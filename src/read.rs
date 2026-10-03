//! Finding entries: through the entry array chain, and by scanning every
//! object in the file for the entries the chain lost.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use crate::bytes::{id_at, u32_at, u64_at};
use crate::compress::{self, Compression, Failure};
use crate::entry::{Entry, Field, Value};
use crate::header::{Header, Id128, State};
use crate::object::{next_offset, Damage, File, Kind, ALIGNMENT, OBJECT_HEADER_SIZE};
use crate::{micros, Journal, Tag};

/// Bytes before an ENTRY object's items: seqnum, realtime, monotonic, boot
/// id, xor hash.
const ENTRY_FIXED: usize = 48;
/// Bytes before an entry array object's items: the next array's offset.
const ARRAY_FIXED: usize = 8;
/// Bytes before a DATA object's payload: hash, next hash, next field,
/// entry, entry array, entry count; compact files add the tail array's
/// offset and count.
const DATA_FIXED: usize = 48;
const DATA_FIXED_COMPACT: usize = 56;
/// Bytes before a FIELD object's name: hash, next hash, head data.
const FIELD_FIXED: usize = 24;
/// Where a FIELD object links its first DATA object, and a DATA object the
/// next of the same field.
const FIELD_HEAD_DATA: usize = 16;
const DATA_NEXT_FIELD: usize = 16;
/// The DATA object flags: XZ, LZ4, ZSTD.
const KNOWN_OBJECT_FLAGS: u8 = 0b111;

/// Read everything after the header.
pub(crate) fn read(data: &[u8], header: Header) -> Journal {
    let mut reader = Reader {
        file: File {
            data,
            arena_start: header.header_size,
            compact: header.is_compact(),
        },
        problems: header_problems(&header, data.len()),
        fields: HashMap::new(),
        names: None,
        field_objects: Vec::new(),
        undecoded: BTreeMap::new(),
        unnamed: 0,
    };
    let scanned = reader.scan();
    reader.field_objects = scanned.fields;
    let entries = reader.entries(&header, scanned.entries);
    for (compression, count) in &reader.undecoded {
        reader.problems.push(format!(
            "{compression}-compressed fields left undecoded (no {compression} decoder): {count}"
        ));
    }
    if reader.unnamed > 0 {
        reader.problems.push(format!(
            "compressed fields left unnamed (no FIELD object lists them): {}",
            reader.unnamed
        ));
    }
    Journal {
        header,
        entries,
        tags: scanned.tags,
        problems: reader.problems,
    }
}

fn header_problems(header: &Header, file_size: usize) -> Vec<String> {
    let mut problems = Vec::new();
    match header.state {
        State::Online => problems.push(
            "state ONLINE: the file was open for writing, its newest entries may be missing"
                .to_owned(),
        ),
        State::Unknown(state) => problems.push(format!("unknown state {state}")),
        State::Offline | State::Archived => {}
    }
    let unknown = header.unknown_incompatible_flags();
    if unknown != 0 {
        problems.push(format!(
            "unknown incompatible flags 0x{unknown:x}, read anyway"
        ));
    }
    let claimed = header.header_size.saturating_add(header.arena_size);
    if (file_size as u64) < claimed {
        problems.push(format!(
            "the file is {file_size} bytes, its header says {claimed}: a truncated copy"
        ));
    }
    problems
}

/// What a scan of every object found.
#[derive(Default)]
struct Scanned {
    entries: Vec<u64>,
    fields: Vec<u64>,
    tags: Vec<Tag>,
}

/// Why a field can't be read.
enum Unreadable {
    Object(Damage),
    Flags(u8),
    Compressed(Compression, &'static str),
    NoEquals,
}

impl fmt::Display for Unreadable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Object(damage) => damage.fmt(f),
            Self::Flags(flags) => write!(f, "unknown compression flags 0x{flags:x}"),
            Self::Compressed(compression, why) => write!(f, "damaged {compression} data: {why}"),
            Self::NoEquals => f.write_str("no '=' between name and value"),
        }
    }
}

struct Reader<'a> {
    file: File<'a>,
    problems: Vec<String>,
    /// DATA objects read so far, by offset; `None` when unreadable.
    fields: HashMap<u64, Option<Arc<Field>>>,
    /// FIELD objects, by offset.
    field_objects: Vec<u64>,
    /// Field names of DATA objects, from the FIELD objects' lists; built
    /// when a payload can't be decompressed to name itself.
    names: Option<HashMap<u64, String>>,
    /// Fields left compressed, per algorithm.
    undecoded: BTreeMap<Compression, usize>,
    /// Fields left compressed that no FIELD object names.
    unnamed: usize,
}

impl Reader<'_> {
    /// Walk every object from the end of the header to the end of the file,
    /// skipping damage to the next object found.
    fn scan(&mut self) -> Scanned {
        let end = self.file.data.len() as u64;
        let mut scanned = Scanned::default();
        // Objects are aligned even when a damaged header's size isn't.
        let mut offset = self.file.arena_start.next_multiple_of(ALIGNMENT);
        while offset < end {
            let object = match self.file.object(offset) {
                Ok(object) => object,
                Err(damage) => {
                    if let Some(found) = self.resync(offset, end) {
                        self.problems.push(format!(
                            "{damage} at offset {offset}: skipped {} bytes to the next object",
                            found - offset
                        ));
                        offset = found;
                        continue;
                    }
                    // Zeros to the end are space allocated but not yet used.
                    if !self.zeros(offset) {
                        self.problems
                            .push(format!("{damage} at offset {offset}: no object after it"));
                    }
                    break;
                }
            };
            match object.kind {
                Kind::Entry => scanned.entries.push(offset),
                Kind::Field => scanned.fields.push(offset),
                Kind::Tag => scanned.tags.push(Tag {
                    offset,
                    seqnum: u64_at(object.body, 0).unwrap_or_default(),
                    epoch: u64_at(object.body, 8).unwrap_or_default(),
                    tag: object.body[16..].try_into().unwrap_or_default(),
                }),
                Kind::Data | Kind::DataHashTable | Kind::FieldHashTable | Kind::EntryArray => {}
            }
            let size = (OBJECT_HEADER_SIZE + object.body.len()) as u64;
            match next_offset(offset, size) {
                Some(next) => offset = next,
                None => break,
            }
        }
        scanned
    }

    /// The next offset after `offset` that holds a plausible object: its
    /// size fits its type and the file, reserved bytes and unknown flags
    /// are clear.
    fn resync(&self, offset: u64, end: u64) -> Option<u64> {
        let data = self.file.data;
        (offset + ALIGNMENT..end)
            .step_by(ALIGNMENT as usize)
            .find(|&candidate| {
                let at = candidate as usize;
                self.file.object(candidate).is_ok()
                    && data[at + 1] & !KNOWN_OBJECT_FLAGS == 0
                    && data[at + 2..at + 8].iter().all(|&b| b == 0)
            })
    }

    /// Whether the file holds only zeros from `offset`: unused space.
    fn zeros(&self, offset: u64) -> bool {
        let rest = self.file.data.get(offset as usize..).unwrap_or_default();
        rest.iter().all(|&b| b == 0)
    }

    /// Entries the chain lists, in its order, then those only the scan
    /// found, in file order.
    fn entries(&mut self, header: &Header, scanned: Vec<u64>) -> Vec<Entry> {
        let mut entries = Vec::new();
        let mut listed = HashSet::new();
        let mut repeated = 0usize;
        for offset in self.chain(header.entry_array_offset) {
            if !listed.insert(offset) {
                repeated += 1;
                continue;
            }
            match self.entry(offset, true) {
                Ok(entry) => entries.push(entry),
                Err(damage) => self.problems.push(format!(
                    "entry at offset {offset}, listed in the entry array chain: {damage}"
                )),
            }
        }
        if repeated > 0 {
            self.problems.push(format!(
                "the entry array chain lists {repeated} entries more than once"
            ));
        }
        if listed.len() as u64 != header.n_entries {
            self.problems.push(format!(
                "the header counts {} entries, the entry array chain lists {}",
                header.n_entries,
                listed.len()
            ));
        }
        let unlisted: Vec<u64> = scanned
            .into_iter()
            .filter(|offset| !listed.contains(offset))
            .collect();
        if !unlisted.is_empty() {
            self.problems.push(format!(
                "{} entries recovered by scanning the file: the entry array chain doesn't list them",
                unlisted.len()
            ));
        }
        for offset in unlisted {
            match self.entry(offset, false) {
                Ok(entry) => entries.push(entry),
                Err(damage) => self
                    .problems
                    .push(format!("entry at offset {offset}: {damage}")),
            }
        }
        entries
    }

    /// Entry offsets in the entry array chain that starts at `offset`.
    fn chain(&mut self, mut offset: u64) -> Vec<u64> {
        let mut entries = Vec::new();
        let mut arrays = HashSet::new();
        while offset != 0 {
            if !arrays.insert(offset) {
                self.problems.push(format!(
                    "the entry array chain loops back to offset {offset}"
                ));
                break;
            }
            let array = match self.file.expect(offset, Kind::EntryArray) {
                Ok(array) => array,
                Err(damage) => {
                    self.problems.push(format!(
                        "entry array chain broken at offset {offset}: {damage}"
                    ));
                    break;
                }
            };
            let (next, items) = array.body.split_at(ARRAY_FIXED);
            // Items past the first zero are unused.
            entries.extend(
                items
                    .chunks_exact(self.file.array_item_size())
                    .map(|item| self.item_offset(item))
                    .take_while(|&entry| entry != 0),
            );
            offset = u64_at(next, 0).unwrap_or_default();
        }
        entries
    }

    /// The offset an entry item or entry array item holds.
    fn item_offset(&self, item: &[u8]) -> u64 {
        if self.file.compact {
            u32_at(item, 0).map(u64::from)
        } else {
            u64_at(item, 0)
        }
        .unwrap_or_default()
    }

    fn entry(&mut self, offset: u64, linked: bool) -> Result<Entry, Damage> {
        let object = self.file.expect(offset, Kind::Entry)?;
        let (fixed, items) = object.body.split_at(ENTRY_FIXED);
        let realtime = u64_at(fixed, 8).unwrap_or_default();
        let fields = items
            .chunks_exact(self.file.entry_item_size())
            .filter_map(|item| self.field(offset, self.item_offset(item)))
            .collect();
        Ok(Entry {
            offset,
            seqnum: u64_at(fixed, 0).unwrap_or_default(),
            realtime,
            time: micros(realtime),
            monotonic: u64_at(fixed, 16).unwrap_or_default(),
            boot_id: Id128(id_at(fixed, 24).unwrap_or_default()),
            xor_hash: u64_at(fixed, 40).unwrap_or_default(),
            linked,
            fields,
        })
    }

    /// The field in the DATA object at `offset`, read once and shared.
    fn field(&mut self, entry: u64, offset: u64) -> Option<Arc<Field>> {
        if let Some(field) = self.fields.get(&offset) {
            return field.clone();
        }
        let field = match self.decode(offset) {
            Ok(field) => Some(Arc::new(field)),
            Err(why) => {
                self.problems.push(format!(
                    "entry at offset {entry}: field at offset {offset}: {why}"
                ));
                None
            }
        };
        self.fields.insert(offset, field.clone());
        field
    }

    fn decode(&mut self, offset: u64) -> Result<Field, Unreadable> {
        let object = self
            .file
            .expect(offset, Kind::Data)
            .map_err(Unreadable::Object)?;
        let fixed = if self.file.compact {
            DATA_FIXED_COMPACT
        } else {
            DATA_FIXED
        };
        let payload = &object.body[fixed..];
        let mut bytes = match compress::from_flags(object.flags).map_err(Unreadable::Flags)? {
            None => payload.to_vec(),
            Some(compression) => match compress::decompress(compression, payload) {
                Ok(bytes) => bytes,
                Err(Failure::Unsupported) => {
                    *self.undecoded.entry(compression).or_default() += 1;
                    let name = self.name_of(offset);
                    if name.is_empty() {
                        self.unnamed += 1;
                    }
                    return Ok(Field {
                        name,
                        value: Value::Compressed {
                            compression,
                            size: payload.len(),
                        },
                    });
                }
                Err(Failure::Damaged(why)) => return Err(Unreadable::Compressed(compression, why)),
            },
        };
        let equals = bytes
            .iter()
            .position(|&b| b == b'=')
            .ok_or(Unreadable::NoEquals)?;
        let value = bytes.split_off(equals + 1);
        bytes.truncate(equals);
        Ok(Field {
            name: String::from_utf8_lossy(&bytes).into_owned(),
            value: Value::Bytes(value),
        })
    }

    /// The name of the field a DATA object belongs to, from the FIELD
    /// objects' lists of their DATA objects; empty when none lists it.
    fn name_of(&mut self, data: u64) -> String {
        let (file, fields) = (self.file, &self.field_objects);
        self.names
            .get_or_insert_with(|| field_names(file, fields))
            .get(&data)
            .cloned()
            .unwrap_or_default()
    }
}

/// Every DATA object's field name, by offset, from each FIELD object's
/// list of its DATA objects.
fn field_names(file: File<'_>, fields: &[u64]) -> HashMap<u64, String> {
    let mut names = HashMap::new();
    for &offset in fields {
        let Ok(field) = file.expect(offset, Kind::Field) else {
            continue;
        };
        let name = String::from_utf8_lossy(&field.body[FIELD_FIXED..]).into_owned();
        let mut data = u64_at(field.body, FIELD_HEAD_DATA).unwrap_or_default();
        // A DATA object already named ends the list: a loop, or damage.
        while data != 0 && !names.contains_key(&data) {
            let Ok(object) = file.expect(data, Kind::Data) else {
                break;
            };
            names.insert(data, name.clone());
            data = u64_at(object.body, DATA_NEXT_FIELD).unwrap_or_default();
        }
    }
    names
}
