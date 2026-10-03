# journal

systemd journal files (`/var/log/journal/*/*.journal`, and the `*.journal~` journald leaves when it finds a file dirty), read from systemd's [documentation of the format](https://systemd.io/JOURNAL_FILE_FORMAT/). One dependency, its sibling `sootmark-common` (times).

```toml
[dependencies]
sootmark-journal = "0.1"
```

```rust
let journal = journal::Journal::parse(&std::fs::read("system.journal")?)?;
for entry in &journal.entries {
    println!("{} {:?} {:?} {:?}", entry.seqnum, entry.time, entry.syslog_identifier(), entry.message());
}
for problem in &journal.problems {
    eprintln!("{problem}");
}
```

## What you get

- `Journal::parse(bytes)`: the header (state, flags, file, machine, boot and sequence ids, counts, head and tail times), every entry, seal tags, and `problems`.
- Each entry: sequence number, wall-clock time (microseconds as written, and as a UTC time), monotonic time and boot id, xor hash, its file offset, and its `NAME=value` fields in stored order. Values are bytes (binary fields stay binary), with lossy text on demand; fields shared by many entries are read once.
- Accessors for the common fields: `message`, `priority`, `syslog_identifier`, `pid`, `uid`, `comm`, `exe`, `cmdline`, `hostname`, `systemd_unit`, `transport`, `source_time` (`_SOURCE_REALTIME_TIMESTAMP`).
- Regular and compact files (32-bit offsets); keyed and unkeyed hashes (entries are read without the hash tables).
- LZ4 payloads decompressed (the block decoder is written here) and Zstandard ones (`sootmark-common`'s decoder; systemd's default since v246), up to 64 MiB a field. XZ payloads are kept as `Value::Compressed` with their size, still named from the FIELD objects, and counted in `problems`.

## Damage is contained

Evidence is hostile. Only a file that isn't a journal or whose header is cut off is an error. Entries are found the way journalctl finds them, through the entry array chain, and also by scanning every object in the file, skipping damage to the next plausible object: entries a broken chain lost, or that an online file's writer never linked, are recovered and marked `linked: false`. Online and truncated copies, entries pointing at unused space, chain loops, impossible sizes and damaged LZ4 data are reported in `problems`, never fatal. Sizes are checked against the file before anything is read or allocated.

## How it's checked

| Check | Result |
|---|---|
| Journals made for this crate (below) against `journalctl -o export --all`: every entry's sequence number, times, boot id and fields, name and value | regular, compact, LZ4 (both), unkeyed, online: all 24 entries match; ZSTD: all match; XZ: all match, the compressed fields by name |
| A truncated copy, and a copy with its chain's first array and 40 arena bytes overwritten (journalctl reads neither) | 14 of 24 entries, exact; all 24 recovered by the scan, one missing the fields its overwritten items pointed at |
| systemd's own test journals (LGPL-2.1, `test/test-journals/`): seven real ones against journalctl, and 623 damaged ones, most by a fuzzer | 10,757 entries match; all 630 read without a panic |
| Property tests: arbitrary bytes, arbitrary objects behind a real header, real files damaged and cut anywhere | no panic (256 cases each per run) |

The journals in `tests/fixtures/generated/` were made for this crate: synthetic entries written in the journal export format (two boots of `lab-host`: sshd logins, failed passwords, a sudo command, a long message, a binary field) and turned into journal files by `systemd-journal-remote` 257, under each of its compact and compression settings. `tests/fixtures/generate.sh` recreates them and the oracle in `tests/oracle/`. They are gzipped: a journal is allocated in large steps, mostly zeros. systemd's test journals are not vendored; CI downloads them at a pinned commit and runs `tests/systemd_samples.rs` on them.

Not yet: XZ payloads, verifying seal tags, and following the hash tables.

## Licence

MIT or Apache-2.0, at your option.
