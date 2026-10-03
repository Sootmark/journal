//! Entries and their fields.

use std::borrow::Cow;
use std::str::FromStr;
use std::sync::Arc;

use common::time::Ts;

use crate::compress::Compression;
use crate::header::Id128;
use crate::micros;

/// One log entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Offset of its ENTRY object in the file.
    pub offset: u64,
    /// Sequence number, within the header's `seqnum_id`.
    pub seqnum: u64,
    /// Wall-clock time as written: microseconds since 1970 (UTC).
    pub realtime: u64,
    /// [`realtime`](Self::realtime) as a time.
    pub time: Ts,
    /// Microseconds since boot.
    pub monotonic: u64,
    /// The boot the monotonic time counts from.
    pub boot_id: Id128,
    /// XOR of its fields' hashes.
    pub xor_hash: u64,
    /// Whether the entry array chain lists it. Entries recovered only by
    /// scanning the file (damaged chain, writer stopped mid-update) are not
    /// listed, and journalctl doesn't show them.
    pub linked: bool,
    /// Its fields, in the order they are stored. A name may repeat.
    pub fields: Vec<Arc<Field>>,
}

/// A `NAME=value` field. Fields are shared by every entry that has them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// The name (`MESSAGE`, `_PID`, …). Empty when the payload couldn't be
    /// decompressed and no FIELD object names it.
    pub name: String,
    /// The value.
    pub value: Value,
}

/// A field's value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// The bytes, which needn't be text.
    Bytes(Vec<u8>),
    /// Compressed with an algorithm this crate doesn't decode.
    Compressed {
        /// The algorithm.
        compression: Compression,
        /// Compressed size of the whole `NAME=value` payload, in bytes.
        size: usize,
    },
}

impl Field {
    /// The value's bytes, when decoded.
    #[must_use]
    pub fn bytes(&self) -> Option<&[u8]> {
        match &self.value {
            Value::Bytes(bytes) => Some(bytes),
            Value::Compressed { .. } => None,
        }
    }

    /// The value as text, invalid UTF-8 replaced, when decoded.
    #[must_use]
    pub fn text(&self) -> Option<Cow<'_, str>> {
        self.bytes().map(String::from_utf8_lossy)
    }
}

impl Entry {
    /// The first field called `name`.
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields
            .iter()
            .map(AsRef::as_ref)
            .find(|f| f.name == name)
    }

    /// The first `name` field's value as text, invalid UTF-8 replaced.
    #[must_use]
    pub fn text(&self, name: &str) -> Option<Cow<'_, str>> {
        self.field(name)?.text()
    }

    /// `MESSAGE`: what was logged.
    #[must_use]
    pub fn message(&self) -> Option<Cow<'_, str>> {
        self.text("MESSAGE")
    }

    /// `PRIORITY`: syslog severity, `0` emergency … `7` debug.
    #[must_use]
    pub fn priority(&self) -> Option<u8> {
        self.number::<u8>("PRIORITY").filter(|&p| p <= 7)
    }

    /// `SYSLOG_IDENTIFIER`: the program's name for itself.
    #[must_use]
    pub fn syslog_identifier(&self) -> Option<Cow<'_, str>> {
        self.text("SYSLOG_IDENTIFIER")
    }

    /// `_PID`: the logging process.
    #[must_use]
    pub fn pid(&self) -> Option<u32> {
        self.number("_PID")
    }

    /// `_UID`: the logging process's user.
    #[must_use]
    pub fn uid(&self) -> Option<u32> {
        self.number("_UID")
    }

    /// `_COMM`: the logging process's name.
    #[must_use]
    pub fn comm(&self) -> Option<Cow<'_, str>> {
        self.text("_COMM")
    }

    /// `_EXE`: the logging process's executable.
    #[must_use]
    pub fn exe(&self) -> Option<Cow<'_, str>> {
        self.text("_EXE")
    }

    /// `_CMDLINE`: the logging process's command line.
    #[must_use]
    pub fn cmdline(&self) -> Option<Cow<'_, str>> {
        self.text("_CMDLINE")
    }

    /// `_HOSTNAME`: the host it was logged on.
    #[must_use]
    pub fn hostname(&self) -> Option<Cow<'_, str>> {
        self.text("_HOSTNAME")
    }

    /// `_SYSTEMD_UNIT`: the logging process's unit.
    #[must_use]
    pub fn systemd_unit(&self) -> Option<Cow<'_, str>> {
        self.text("_SYSTEMD_UNIT")
    }

    /// `_TRANSPORT`: how it reached the journal (`journal`, `syslog`,
    /// `kernel`, `stdout`, `audit`, `driver`).
    #[must_use]
    pub fn transport(&self) -> Option<Cow<'_, str>> {
        self.text("_TRANSPORT")
    }

    /// `_SOURCE_REALTIME_TIMESTAMP`: when the program says it logged, which
    /// may precede [`time`](Self::time).
    #[must_use]
    pub fn source_time(&self) -> Option<Ts> {
        self.number("_SOURCE_REALTIME_TIMESTAMP").map(micros)
    }

    fn number<T: FromStr>(&self, name: &str) -> Option<T> {
        self.text(name)?.parse().ok()
    }
}
