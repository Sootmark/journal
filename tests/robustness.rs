//! Damaged and hostile input: read what is there, report the rest, never
//! panic.

mod support;

use journal::{Error, Journal};
use proptest::prelude::*;
use support::{decoded_fields, exported_fields, fixture, number, oracle, Exported};

/// Where the header keeps the first entry array's offset.
const ENTRY_ARRAY_OFFSET: usize = 176;

/// Read everything, accessors included. Panics are the only failure.
fn read_all(bytes: &[u8]) {
    if let Ok(journal) = Journal::parse(bytes) {
        for entry in &journal.entries {
            let _ = (
                entry.message(),
                entry.priority(),
                entry.pid(),
                entry.source_time(),
            );
            for field in &entry.fields {
                let _ = field.text();
            }
        }
    }
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

/// The exported entry with `seqnum`.
fn exported(oracle: &[Exported], seqnum: u64) -> &Exported {
    oracle
        .iter()
        .find(|e| number(e, "__SEQNUM") == seqnum)
        .unwrap()
}

#[test]
fn rejects_what_is_not_a_journal() {
    assert_eq!(Journal::parse(b"").unwrap_err(), Error::NotJournal);
    assert_eq!(
        Journal::parse(b"MZ\x90\0 not a journal").unwrap_err(),
        Error::NotJournal
    );
    let regular = fixture("regular.journal");
    assert_eq!(
        Journal::parse(&regular[..100]).unwrap_err(),
        Error::TruncatedHeader(100)
    );
    let mut small = regular.clone();
    small[88..96].copy_from_slice(&16u64.to_le_bytes());
    assert_eq!(
        Journal::parse(&small).unwrap_err(),
        Error::BadHeaderSize(16)
    );
}

#[test]
fn a_truncated_copy_yields_its_whole_entries() {
    let journal = Journal::parse(&fixture("truncated.journal")).unwrap();
    let expected = oracle("regular");
    assert!(journal.entries.len() >= 10, "{}", journal.entries.len());
    assert!(journal.entries.len() < expected.len());
    for (entry, exported) in journal.entries.iter().zip(&expected) {
        assert!(entry.linked);
        assert_eq!(decoded_fields(entry), exported_fields(exported));
    }
    assert!(journal.problems[0].ends_with("a truncated copy"));
    let lost = expected.len() - journal.entries.len();
    let past_end = journal
        .problems
        .iter()
        .filter(|p| p.ends_with("past the end of the file"));
    assert_eq!(past_end.count(), lost);
}

#[test]
fn a_corrupt_chain_falls_back_to_the_scan() {
    let journal = Journal::parse(&fixture("corrupt.journal~")).unwrap();
    let expected = oracle("compact-lz4");
    assert_eq!(journal.entries.len(), expected.len());
    let mut damaged = 0;
    for entry in &journal.entries {
        assert!(!entry.linked);
        let theirs = exported_fields(exported(&expected, entry.seqnum));
        let ours = decoded_fields(entry);
        if ours != theirs {
            damaged += 1;
            assert!(ours.iter().all(|field| theirs.contains(field)));
        }
    }
    assert_eq!(damaged, 1, "the entry whose items were overwritten");
    let problems = journal.problems.join("\n");
    assert!(problems.contains("entry array chain broken at offset"));
    assert!(problems.contains("24 entries recovered by scanning the file"));
}

#[test]
fn an_entry_array_loop_ends() {
    let mut bytes = fixture("regular.journal");
    let first = u64_at(&bytes, ENTRY_ARRAY_OFFSET) as usize;
    bytes[first + 16..first + 24].copy_from_slice(&(first as u64).to_le_bytes());
    let journal = Journal::parse(&bytes).unwrap();
    assert_eq!(journal.entries.len(), 24, "the rest found by scanning");
    assert!(journal.problems.iter().any(|p| p.contains("loops back")));
}

#[test]
fn damaged_lz4_data_loses_only_its_field() {
    let mut bytes = fixture("lz4.journal");
    let header_size = u64_at(&bytes, 88) as usize;
    // The LZ4 DATA object: type 1, flags 2.
    let data = (header_size..bytes.len())
        .step_by(8)
        .find(|&at| bytes[at..at + 2] == [1, 2])
        .unwrap();
    let block = data + 16 + 48 + 8;
    bytes[block..block + 4].copy_from_slice(&[0xff; 4]);
    let journal = Journal::parse(&bytes).unwrap();
    assert_eq!(journal.entries.len(), 24);
    assert_eq!(journal.problems.len(), 1, "{:?}", journal.problems);
    assert!(journal.problems[0].contains("damaged LZ4 data"));
    let lost = journal
        .entries
        .iter()
        .filter(|e| e.message().is_none())
        .count();
    assert_eq!(lost, 2, "both boots log the same long message");
}

proptest! {
    #[test]
    fn arbitrary_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        read_all(&bytes);
    }

    #[test]
    fn arbitrary_objects_behind_a_header(body in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let mut bytes = fixture("compact.journal")[..272].to_vec();
        bytes.extend(body);
        read_all(&bytes);
    }

    #[test]
    fn damaged_real_files(
        name in prop::sample::select(vec!["regular.journal", "compact-lz4.journal", "lz4.journal"]),
        damage in proptest::collection::vec((0usize..70_000, any::<u8>()), 1..64),
        length in 0usize..80_000,
    ) {
        let mut bytes = fixture(name);
        for (at, byte) in damage {
            bytes[at] = byte;
        }
        bytes.truncate(length.max(1_000));
        read_all(&bytes);
    }
}
