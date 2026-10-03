//! The file header.

use core::fmt;

use common::hex;
use common::time::Ts;

use crate::bytes::{id_at, u32_at, u64_at};
use crate::error::Error;
use crate::micros;

const SIGNATURE: &[u8; 8] = b"LPKSHHRH";
/// The header's size up to `tail_entry_monotonic`, its oldest layout; the
/// counters after it were added in systemd 187 and later.
pub(crate) const OLDEST_HEADER_SIZE: u64 = 208;

const COMPATIBLE_SEALED: u32 = 1 << 0;
const INCOMPATIBLE_KEYED_HASH: u32 = 1 << 2;
const INCOMPATIBLE_COMPACT: u32 = 1 << 4;
/// Every incompatible flag this reader knows: XZ, LZ4, keyed hash, ZSTD,
/// compact.
const INCOMPATIBLE_KNOWN: u32 = 0b1_1111;

/// A 128-bit id (file, machine, boot, sequence number id).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Id128(pub [u8; 16]);

impl fmt::Display for Id128 {
    /// 32 lowercase hex digits, as journalctl writes it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(&self.0))
    }
}

/// Whether the file was closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Closed cleanly.
    Offline,
    /// Open for writing: copied from a live system, or its writer died.
    Online,
    /// Closed and rotated away.
    Archived,
    /// A value systemd doesn't define.
    Unknown(u8),
}

/// The file header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// Features a reader may ignore (bit 0: sealed).
    pub compatible_flags: u32,
    /// Features a reader must understand (bits 0, 1, 3: XZ, LZ4, ZSTD data;
    /// bit 2: keyed hash; bit 4: compact).
    pub incompatible_flags: u32,
    /// Whether the file was closed.
    pub state: State,
    /// This file's id.
    pub file_id: Id128,
    /// The machine that wrote it.
    pub machine_id: Id128,
    /// Boot of the newest entry (or, in old files, of the writer).
    pub tail_entry_boot_id: Id128,
    /// The sequence the entries' numbers belong to.
    pub seqnum_id: Id128,
    /// Size of this header in bytes.
    pub header_size: u64,
    /// Size of the object area after the header.
    pub arena_size: u64,
    /// Offset of the newest object.
    pub tail_object_offset: u64,
    /// Objects the header counts.
    pub n_objects: u64,
    /// Entries the header counts.
    pub n_entries: u64,
    /// Sequence number of the oldest entry.
    pub head_entry_seqnum: u64,
    /// Sequence number of the newest entry.
    pub tail_entry_seqnum: u64,
    /// Offset of the first array of the entry array chain.
    pub entry_array_offset: u64,
    /// Wall-clock time of the oldest entry (UTC).
    pub head_time: Ts,
    /// Wall-clock time of the newest entry (UTC).
    pub tail_time: Ts,
    /// Microseconds since boot at the newest entry.
    pub tail_entry_monotonic: u64,
}

impl Header {
    /// Whether entry items and entry arrays hold 32-bit offsets.
    #[must_use]
    pub const fn is_compact(&self) -> bool {
        self.incompatible_flags & INCOMPATIBLE_COMPACT != 0
    }

    /// Whether hash tables use a keyed (SipHash) hash.
    #[must_use]
    pub const fn has_keyed_hash(&self) -> bool {
        self.incompatible_flags & INCOMPATIBLE_KEYED_HASH != 0
    }

    /// Whether the file carries forward-secure seal tags.
    #[must_use]
    pub const fn is_sealed(&self) -> bool {
        self.compatible_flags & COMPATIBLE_SEALED != 0
    }

    /// Incompatible flags this reader doesn't know.
    #[must_use]
    pub const fn unknown_incompatible_flags(&self) -> u32 {
        self.incompatible_flags & !INCOMPATIBLE_KNOWN
    }

    pub(crate) fn parse(data: &[u8]) -> Result<Self, Error> {
        if data.len() < SIGNATURE.len() || &data[..SIGNATURE.len()] != SIGNATURE {
            return Err(Error::NotJournal);
        }
        let truncated = Error::TruncatedHeader(data.len());
        let header_size = u64_at(data, 88).ok_or_else(|| truncated.clone())?;
        if header_size < OLDEST_HEADER_SIZE {
            return Err(Error::BadHeaderSize(header_size));
        }
        if header_size > data.len() as u64 {
            return Err(truncated);
        }
        // Every read below is within the oldest layout, now known present.
        let u64_at = |at| u64_at(data, at).unwrap_or_default();
        let id_at = |at| Id128(id_at(data, at).unwrap_or_default());
        Ok(Self {
            compatible_flags: u32_at(data, 8).unwrap_or_default(),
            incompatible_flags: u32_at(data, 12).unwrap_or_default(),
            state: match data[16] {
                0 => State::Offline,
                1 => State::Online,
                2 => State::Archived,
                other => State::Unknown(other),
            },
            file_id: id_at(24),
            machine_id: id_at(40),
            tail_entry_boot_id: id_at(56),
            seqnum_id: id_at(72),
            header_size,
            arena_size: u64_at(96),
            tail_object_offset: u64_at(136),
            n_objects: u64_at(144),
            n_entries: u64_at(152),
            tail_entry_seqnum: u64_at(160),
            head_entry_seqnum: u64_at(168),
            entry_array_offset: u64_at(176),
            head_time: micros(u64_at(184)),
            tail_time: micros(u64_at(192)),
            tail_entry_monotonic: u64_at(200),
        })
    }
}
