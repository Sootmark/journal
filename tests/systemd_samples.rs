//! systemd's own test journals (`test/test-journals/` in its repository,
//! LGPL-2.1, so not vendored here): real journals of a test machine, and
//! journals corrupted by a fuzzer.
//!
//! Set `SOOTMARK_JOURNAL_SYSTEMD` to a folder of them, decompressed; CI
//! downloads them at the commit `.github/workflows/ci.yml` names. A file
//! with a `NAME.export` beside it (journalctl's `-o export --all`, which CI
//! writes) must match it entry for entry when journalctl could read it;
//! every file must read without a panic.

mod support;

use std::path::{Path, PathBuf};

use journal::Journal;
use support::{assert_exported, number, parse_export};

const FOLDER: &str = "SOOTMARK_JOURNAL_SYSTEMD";

fn files(folder: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files(&path, found);
        } else if path.extension() != Some("export".as_ref()) {
            found.push(path);
        }
    }
}

#[test]
fn every_systemd_sample() {
    let Some(folder) = std::env::var_os(FOLDER) else {
        eprintln!("{FOLDER} not set: skipped");
        return;
    };
    let mut found = Vec::new();
    files(Path::new(&folder), &mut found);
    assert!(!found.is_empty(), "no files in {folder:?}");
    let mut compared = 0;
    for path in found {
        let Ok(journal) = Journal::parse(&std::fs::read(&path).unwrap()) else {
            continue;
        };
        let export = path.with_file_name(format!(
            "{}.export",
            path.file_name().unwrap().to_string_lossy()
        ));
        // journalctl refuses some damaged files this crate reads.
        let export = std::fs::read(export).unwrap_or_default();
        if export.is_empty() {
            continue;
        }
        let exported = parse_export(&export);
        let linked = journal.entries.iter().filter(|e| e.linked).count();
        assert_eq!(linked, exported.len(), "{}", path.display());
        for exported in exported {
            let seqnum = number(&exported, "__SEQNUM");
            let at = format!("{} seqnum {seqnum}", path.display());
            let entry = journal.entries.iter().find(|e| e.seqnum == seqnum);
            assert_exported(
                entry.unwrap_or_else(|| panic!("{at}: missing")),
                &exported,
                &at,
            );
            compared += 1;
        }
    }
    eprintln!("{compared} entries match journalctl");
}
