//! Compressed DATA payloads.

use core::fmt;

use crate::bytes::u64_at;
use crate::lz4;

/// Largest field value decompressed: well above anything journald logs
/// but a core dump, and a bound on what hostile data can make us allocate.
const MAX_FIELD_SIZE: usize = 64 << 20;

/// DATA object flags naming the compression.
const XZ: u8 = 1 << 0;
const LZ4: u8 = 1 << 1;
const ZSTD: u8 = 1 << 2;

/// How a DATA object's payload is compressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Compression {
    /// XZ (LZMA2).
    Xz,
    /// LZ4 block, after a little-endian 64-bit decompressed size.
    Lz4,
    /// Zstandard frame.
    Zstd,
}

impl fmt::Display for Compression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Xz => "XZ",
            Self::Lz4 => "LZ4",
            Self::Zstd => "ZSTD",
        })
    }
}

/// Why a payload wasn't decompressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Failure {
    /// No decoder for this algorithm here.
    Unsupported,
    /// The compressed data is damaged.
    Damaged(&'static str),
}

/// The compression a DATA object's flags name: `Ok(None)` for none, an
/// error for flags that name several or unknown ones.
pub(crate) fn from_flags(flags: u8) -> Result<Option<Compression>, u8> {
    match flags {
        0 => Ok(None),
        XZ => Ok(Some(Compression::Xz)),
        LZ4 => Ok(Some(Compression::Lz4)),
        ZSTD => Ok(Some(Compression::Zstd)),
        other => Err(other),
    }
}

/// Decompress a DATA payload.
pub(crate) fn decompress(compression: Compression, payload: &[u8]) -> Result<Vec<u8>, Failure> {
    match compression {
        Compression::Lz4 => {
            let size = u64_at(payload, 0).ok_or(Failure::Damaged("no size before the block"))?;
            let size = usize::try_from(size).map_err(|_| Failure::Damaged("impossible size"))?;
            lz4::decompress(&payload[8..], size).map_err(Failure::Damaged)
        }
        Compression::Zstd => common::zstd::decompress(payload, MAX_FIELD_SIZE)
            .map_err(|_| Failure::Damaged("not a valid Zstandard frame, or over 64 MiB")),
        Compression::Xz => Err(Failure::Unsupported),
    }
}
