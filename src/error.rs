//! Why a file can't be read at all.

use core::fmt;

/// A file that isn't a journal, or whose header can't be read. Damage past
/// the header is never an error: it goes to [`Journal::problems`].
///
/// [`Journal::problems`]: crate::Journal::problems
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The file doesn't start with `LPKSHHRH`.
    NotJournal,
    /// The file ends inside its header: this many bytes are present.
    TruncatedHeader(usize),
    /// The header gives its own size as this, smaller than its oldest
    /// layout.
    BadHeaderSize(u64),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotJournal => f.write_str("not a journal file (bad signature)"),
            Self::TruncatedHeader(length) => {
                write!(f, "file ends inside its header ({length} bytes)")
            }
            Self::BadHeaderSize(size) => write!(f, "impossible header size {size}"),
        }
    }
}

impl std::error::Error for Error {}
