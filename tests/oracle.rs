//! Every entry of every generated journal against journalctl's export of
//! it: sequence number, times, boot, and every field, name and value.

mod support;

use std::collections::HashSet;
use std::sync::Arc;

use journal::{Compression, Journal, State, Value};
use support::{assert_exported, fixture, oracle, value};

/// `fixture`'s entries, as journalctl exported `export`.
fn matches_oracle(fixture_name: &str, export: &str) -> Journal {
    let journal = Journal::parse(&fixture(fixture_name)).unwrap();
    let expected = oracle(export);
    assert_eq!(journal.entries.len(), expected.len(), "{fixture_name}");
    for (entry, exported) in journal.entries.iter().zip(&expected) {
        let at = format!("{fixture_name} seqnum {}", entry.seqnum);
        assert!(entry.linked, "{at}");
        assert_exported(entry, exported, &at);
        assert_eq!(
            journal.header.seqnum_id.to_string().as_bytes(),
            value(exported, "__SEQNUM_ID"),
            "{at}"
        );
    }
    journal
}

/// Distinct fields left compressed with `compression`.
fn compressed(journal: &Journal, compression: Compression) -> usize {
    let fields = journal.entries.iter().flat_map(|e| &e.fields);
    fields
        .filter(|f| matches!(f.value, Value::Compressed { compression: c, .. } if c == compression))
        .map(Arc::as_ptr)
        .collect::<HashSet<_>>()
        .len()
}

#[test]
fn regular() {
    let journal = matches_oracle("regular.journal", "regular");
    assert!(journal.problems.is_empty(), "{:?}", journal.problems);
    assert!(!journal.header.is_compact());
    assert!(journal.header.has_keyed_hash());
    assert_eq!(journal.header.state, State::Offline);
    assert_eq!(
        journal.header.machine_id.to_string(),
        "5d0c7f4a0b8e4c51a3a1b7e0c2d9f611"
    );
}

#[test]
fn compact() {
    let journal = matches_oracle("compact.journal", "compact");
    assert!(journal.problems.is_empty(), "{:?}", journal.problems);
    assert!(journal.header.is_compact());
}

#[test]
fn unkeyed_hash() {
    let journal = matches_oracle("unkeyed.journal", "unkeyed");
    assert!(journal.problems.is_empty(), "{:?}", journal.problems);
    assert!(!journal.header.has_keyed_hash());
}

#[test]
fn lz4_decoded() {
    for name in ["lz4", "compact-lz4"] {
        let journal = matches_oracle(&format!("{name}.journal"), name);
        assert!(
            journal.problems.is_empty(),
            "{name}: {:?}",
            journal.problems
        );
        let long = journal.entries.iter().find_map(|e| {
            let message = e.message()?;
            (message.len() > 512).then_some(message.into_owned())
        });
        assert!(
            long.is_some_and(|m| m.starts_with("line 000: backup")),
            "{name}"
        );
    }
}

#[test]
fn zstd_decoded() {
    let journal = matches_oracle("zstd.journal", "zstd");
    assert!(journal.problems.is_empty(), "{:?}", journal.problems);
    assert_eq!(compressed(&journal, Compression::Zstd), 0);
    assert!(journal
        .entries
        .iter()
        .filter_map(journal::Entry::message)
        .any(|m| m.len() > 512 && m.starts_with("line 000: backup")));
}

#[test]
fn xz_kept_compressed_and_named() {
    let journal = matches_oracle("xz.journal", "xz");
    let count = compressed(&journal, Compression::Xz);
    assert!(count > 0);
    assert_eq!(
        journal.problems,
        [format!(
            "XZ-compressed fields left undecoded (no XZ decoder): {count}"
        )]
    );
}

#[test]
fn online_file_is_read_and_reported() {
    let journal = matches_oracle("online.journal", "compact");
    assert_eq!(journal.header.state, State::Online);
    assert_eq!(journal.problems.len(), 1);
    assert!(journal.problems[0].starts_with("state ONLINE"));
}

#[test]
fn convenience_accessors() {
    let journal = Journal::parse(&fixture("regular.journal")).unwrap();
    let sudo = journal
        .entries
        .iter()
        .find(|e| e.syslog_identifier().as_deref() == Some("sudo"))
        .unwrap();
    assert_eq!(
        sudo.message().unwrap(),
        "alice : TTY=pts/0 ; PWD=/home/alice ; USER=root ; COMMAND=/usr/bin/systemctl restart nginx"
    );
    assert_eq!(sudo.priority(), Some(5));
    assert_eq!(sudo.pid(), Some(1290));
    assert_eq!(sudo.uid(), Some(1000));
    assert_eq!(sudo.comm().unwrap(), "sudo");
    assert_eq!(sudo.exe().unwrap(), "/usr/bin/sudo");
    assert_eq!(sudo.cmdline().unwrap(), "sudo systemctl restart nginx");
    assert_eq!(sudo.hostname().unwrap(), "lab-host");
    assert_eq!(sudo.systemd_unit().unwrap(), "session-3.scope");
    assert_eq!(sudo.transport().unwrap(), "syslog");
    let source = sudo.source_time().unwrap().ticks().unwrap();
    assert_eq!(source, (sudo.realtime as i64 - 5) * 10);
    assert_eq!(
        sudo.time.to_iso8601().unwrap()[..19],
        *"2023-11-14T22:13:27"
    );

    let crash = journal
        .entries
        .iter()
        .find(|e| e.comm().as_deref() == Some("crasher"))
        .unwrap();
    let payload = crash.field("PAYLOAD").unwrap().bytes().unwrap();
    assert_eq!(&payload[..256], (0..=255).collect::<Vec<u8>>());
    assert!(crash.text("PAYLOAD").unwrap().contains('\u{fffd}'));

    let boots: std::collections::BTreeSet<_> = journal.entries.iter().map(|e| e.boot_id).collect();
    assert_eq!(boots.len(), 2);
}
