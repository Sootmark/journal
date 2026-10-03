//! Fixtures and journalctl's export of them.

// Each test crate uses its own part of this module.
#![allow(dead_code)]

use std::io::Read;

/// A generated fixture, decompressed (`tests/fixtures/generated/NAME.gz`).
pub fn fixture(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/tests/fixtures/generated/{name}.gz",
        env!("CARGO_MANIFEST_DIR")
    );
    let file = std::fs::File::open(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut bytes = Vec::new();
    common::gzip::Decoder::new(file)
        .read_to_end(&mut bytes)
        .unwrap();
    bytes
}

/// One entry of `journalctl -o export`: its fields in order, `__` ones
/// included.
pub type Exported = Vec<(String, Vec<u8>)>;

/// journalctl's export of a fixture (`tests/oracle/NAME.export`).
pub fn oracle(name: &str) -> Vec<Exported> {
    let path = format!("{}/tests/oracle/{name}.export", env!("CARGO_MANIFEST_DIR"));
    parse_export(&std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}")))
}

/// The value of `name` in an exported entry.
pub fn value<'a>(entry: &'a Exported, name: &str) -> &'a [u8] {
    &entry.iter().find(|(n, _)| n == name).unwrap().1
}

/// A number in an exported entry.
pub fn number(entry: &Exported, name: &str) -> u64 {
    std::str::from_utf8(value(entry, name))
        .unwrap()
        .parse()
        .unwrap()
}

/// The export format: `NAME=value` lines, or `NAME`, a little-endian 64-bit
/// length and that many bytes for binary values; a blank line ends an entry.
pub fn parse_export(mut data: &[u8]) -> Vec<Exported> {
    let mut entries = Vec::new();
    let mut entry = Vec::new();
    while !data.is_empty() {
        let end = data.iter().position(|&b| b == b'\n').unwrap();
        let line = &data[..end];
        data = &data[end + 1..];
        if line.is_empty() {
            entries.push(std::mem::take(&mut entry));
        } else if let Some(equals) = line.iter().position(|&b| b == b'=') {
            let name = String::from_utf8(line[..equals].to_vec()).unwrap();
            entry.push((name, line[equals + 1..].to_vec()));
        } else {
            let length = u64::from_le_bytes(data[..8].try_into().unwrap()) as usize;
            entry.push((
                String::from_utf8(line.to_vec()).unwrap(),
                data[8..8 + length].to_vec(),
            ));
            data = &data[8 + length + 1..];
        }
    }
    entries
}

/// An exported entry's fields, without journalctl's `__` ones and the boot
/// id, which it writes from the entry rather than from its field.
pub fn exported_fields(entry: &Exported) -> Vec<(&str, &[u8])> {
    entry
        .iter()
        .filter(|(name, _)| !name.starts_with("__") && name != "_BOOT_ID")
        .map(|(name, value)| (name.as_str(), value.as_slice()))
        .collect()
}

/// An entry's decoded fields, its `_BOOT_ID` left out as journalctl does.
pub fn decoded_fields(entry: &journal::Entry) -> Vec<(&str, &[u8])> {
    entry
        .fields
        .iter()
        .filter(|f| f.name != "_BOOT_ID")
        .filter_map(|f| Some((f.name.as_str(), f.bytes()?)))
        .collect()
}

/// Assert `entry` is what journalctl exported: sequence number, times,
/// boot, and every field in order, those left compressed by name only.
pub fn assert_exported(entry: &journal::Entry, exported: &Exported, at: &str) {
    assert_eq!(entry.seqnum, number(exported, "__SEQNUM"), "{at}");
    assert_eq!(
        entry.realtime,
        number(exported, "__REALTIME_TIMESTAMP"),
        "{at}"
    );
    assert_eq!(
        entry.monotonic,
        number(exported, "__MONOTONIC_TIMESTAMP"),
        "{at}"
    );
    let boot_id = value(exported, "_BOOT_ID");
    assert_eq!(entry.boot_id.to_string().as_bytes(), boot_id, "{at}");
    // journalctl writes the boot id from the entry, not from its field.
    if let Some(field) = entry.field("_BOOT_ID") {
        assert_eq!(field.bytes(), Some(boot_id), "{at}");
    }
    let ours: Vec<_> = entry
        .fields
        .iter()
        .filter(|f| f.name != "_BOOT_ID")
        .collect();
    let theirs = exported_fields(exported);
    assert_eq!(ours.len(), theirs.len(), "{at}");
    for (field, (name, bytes)) in ours.into_iter().zip(theirs) {
        match field.bytes() {
            Some(ours) => {
                assert_eq!(field.name, name, "{at}");
                assert_eq!(ours, bytes, "{at} {name}");
            }
            // Compressed: named by its FIELD object, if one lists it.
            None => assert!(field.name == name || field.name.is_empty(), "{at} {name}"),
        }
    }
}
