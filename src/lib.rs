//! systemd journal files (`*.journal`, and `*.journal~`, the name journald
//! gives a file it found dirty), read from the format systemd documents
//! (<https://systemd.io/JOURNAL_FILE_FORMAT/>).
//!
//! ```no_run
//! let journal = journal::Journal::parse(&std::fs::read("system.journal")?)?;
//! for entry in &journal.entries {
//!     println!("{} {:?} {:?}", entry.seqnum, entry.time, entry.message());
//! }
//! for problem in &journal.problems {
//!     eprintln!("{problem}");
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! Entries are found through the entry array chain, as journalctl finds
//! them, and by scanning every object in the file: entries the chain lost
//! to damage, or never got because the writer stopped, are recovered and
//! marked unlinked. Regular and compact files are read; LZ4 payloads are
//! decompressed, XZ and Zstandard ones kept compressed and reported.
//!
//! Damage is contained: only a file that isn't a journal, or whose header
//! is cut off, is an [`Error`]. Everything else is read as far as it goes,
//! with what couldn't be in [`Journal::problems`]; hostile input never
//! panics.

use common::time::Ts;

mod bytes;
mod compress;
mod entry;
mod error;
mod header;
mod lz4;
mod object;
mod read;

pub use compress::Compression;
pub use entry::{Entry, Field, Value};
pub use error::Error;
pub use header::{Header, Id128, State};

/// This crate's version, for records of what parsed them.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A journal file's contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Journal {
    /// The file header.
    pub header: Header,
    /// Entries the entry array chain lists, in its order (oldest first),
    /// then entries only a scan of the file found, in file order.
    pub entries: Vec<Entry>,
    /// Seal tags, in file order, unverified.
    pub tags: Vec<Tag>,
    /// What couldn't be read, and what the file says about its own state.
    pub problems: Vec<String>,
}

/// A forward-secure seal tag: an HMAC over the objects before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    /// Offset of its TAG object in the file.
    pub offset: u64,
    /// Its sequence number.
    pub seqnum: u64,
    /// The sealing key's epoch.
    pub epoch: u64,
    /// The HMAC-SHA256.
    pub tag: [u8; 32],
}

impl Journal {
    /// Read a journal file.
    ///
    /// # Errors
    /// [`Error`] when the file isn't a journal or its header is cut off.
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        Ok(read::read(data, header::Header::parse(data)?))
    }
}

/// Microseconds since 1970 as a time; past `i64` it overflows to invalid.
fn micros(value: u64) -> Ts {
    Ts::from_unix_micros(i64::try_from(value).unwrap_or(i64::MAX))
}
