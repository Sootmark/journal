//! Objects: the 16-byte object header, and the checks every object passes
//! before its body is read.

use core::fmt;

use crate::bytes::u64_at;

/// Size of the object header (type, flags, reserved, size).
pub(crate) const OBJECT_HEADER_SIZE: usize = 16;
/// Objects start on 8-byte boundaries.
pub(crate) const ALIGNMENT: u64 = 8;

/// An object's type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Data,
    Field,
    Entry,
    DataHashTable,
    FieldHashTable,
    EntryArray,
    Tag,
}

impl Kind {
    const fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            1 => Self::Data,
            2 => Self::Field,
            3 => Self::Entry,
            4 => Self::DataHashTable,
            5 => Self::FieldHashTable,
            6 => Self::EntryArray,
            7 => Self::Tag,
            _ => return None,
        })
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Data => "DATA",
            Self::Field => "FIELD",
            Self::Entry => "ENTRY",
            Self::DataHashTable => "DATA_HASH_TABLE",
            Self::FieldHashTable => "FIELD_HASH_TABLE",
            Self::EntryArray => "ENTRY_ARRAY",
            Self::Tag => "TAG",
        }
    }
}

/// A checked object.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Object<'a> {
    pub kind: Kind,
    pub flags: u8,
    /// Everything after the object header, `size - 16` bytes.
    pub body: &'a [u8],
}

/// Why there is no readable object at an offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Damage {
    /// Not on an 8-byte boundary, or inside the file header.
    Misplaced,
    /// The file ends before the object (or inside its header).
    PastEnd,
    /// A type byte systemd doesn't define (0 is unused space).
    UnknownType(u8),
    /// A size impossible for the type.
    BadSize(Kind, u64),
    /// The file ends inside the object.
    Truncated(Kind, u64),
    /// Another type than expected.
    Unexpected(Kind),
}

impl fmt::Display for Damage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Misplaced => f.write_str("misplaced offset"),
            Self::PastEnd => f.write_str("past the end of the file"),
            Self::UnknownType(0) => f.write_str("unused space"),
            Self::UnknownType(byte) => write!(f, "unknown object type {byte}"),
            Self::BadSize(kind, size) => {
                write!(f, "{} object of impossible size {size}", kind.name())
            }
            Self::Truncated(kind, size) => write!(
                f,
                "{} object of {size} bytes cut off by the end of the file",
                kind.name()
            ),
            Self::Unexpected(kind) => write!(f, "unexpected {} object", kind.name()),
        }
    }
}

/// The journal's bytes, for reading objects.
#[derive(Debug, Clone, Copy)]
pub(crate) struct File<'a> {
    pub data: &'a [u8],
    /// Where objects may start: the end of the file header.
    pub arena_start: u64,
    /// Whether offsets in entries and entry arrays are 32-bit.
    pub compact: bool,
}

impl<'a> File<'a> {
    /// Bytes in an entry item: an offset, and in regular files its hash.
    pub const fn entry_item_size(self) -> usize {
        if self.compact {
            4
        } else {
            16
        }
    }

    /// Bytes in an entry array item.
    pub const fn array_item_size(self) -> usize {
        if self.compact {
            4
        } else {
            8
        }
    }

    /// The object at `offset`, which must be of `kind`.
    pub fn expect(self, offset: u64, kind: Kind) -> Result<Object<'a>, Damage> {
        let object = self.object(offset)?;
        if object.kind == kind {
            Ok(object)
        } else {
            Err(Damage::Unexpected(object.kind))
        }
    }

    /// The object at `offset`, its size checked against its type and the
    /// file.
    pub fn object(self, offset: u64) -> Result<Object<'a>, Damage> {
        if offset < self.arena_start || offset % ALIGNMENT != 0 {
            return Err(Damage::Misplaced);
        }
        let start = usize::try_from(offset).map_err(|_| Damage::PastEnd)?;
        let head = self
            .data
            .get(start..)
            .and_then(|rest| rest.get(..OBJECT_HEADER_SIZE))
            .ok_or(Damage::PastEnd)?;
        let kind = Kind::from_byte(head[0]).ok_or(Damage::UnknownType(head[0]))?;
        let size = u64_at(head, 8).unwrap_or_default();
        if !self.fits(kind, size) {
            return Err(Damage::BadSize(kind, size));
        }
        let available = (self.data.len() - start) as u64;
        if size > available {
            return Err(Damage::Truncated(kind, size));
        }
        Ok(Object {
            kind,
            flags: head[1],
            body: &self.data[start + OBJECT_HEADER_SIZE..start + size as usize],
        })
    }

    /// Whether `size` is possible for an object of `kind`.
    fn fits(self, kind: Kind, size: u64) -> bool {
        let (fixed, item) = match kind {
            Kind::Data if self.compact => (72, 1),
            Kind::Data => (64, 1),
            Kind::Field => (40, 1),
            Kind::Entry => (64, self.entry_item_size()),
            Kind::DataHashTable | Kind::FieldHashTable => (16, 16),
            Kind::EntryArray => (24, self.array_item_size()),
            Kind::Tag => return size == 64,
        };
        size >= fixed && (size - fixed) % item as u64 == 0
    }
}

/// The offset after an object of `size` bytes at `offset`: the next 8-byte
/// boundary.
pub(crate) fn next_offset(offset: u64, size: u64) -> Option<u64> {
    offset
        .checked_add(size)?
        .checked_next_multiple_of(ALIGNMENT)
}
