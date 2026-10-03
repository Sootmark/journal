# Generated journals

Made for this crate by `generate.sh` (systemd 257, Debian 13): synthetic entries only, two boots of `lab-host`, written in the journal export format and turned into journal files by `systemd-journal-remote`. Gzipped, since a journal file is mostly allocated zeros. journalctl's export of each is in `../oracle/`.

| File | Made with |
|---|---|
| `regular` | `SYSTEMD_JOURNAL_COMPACT=0 SYSTEMD_JOURNAL_COMPRESS=NONE` |
| `compact` | `SYSTEMD_JOURNAL_COMPACT=1 SYSTEMD_JOURNAL_COMPRESS=NONE` |
| `lz4`, `compact-lz4` | `SYSTEMD_JOURNAL_COMPRESS=LZ4`, regular and compact |
| `zstd` | `SYSTEMD_JOURNAL_COMPRESS=ZSTD`, compact |
| `xz` | `SYSTEMD_JOURNAL_COMPRESS=XZ`, regular |
| `unkeyed` | `SYSTEMD_JOURNAL_KEYED_HASH=0` |
| `online` | `compact` with its state set to ONLINE |
| `truncated` | `regular` cut two thirds of the way into its objects |
| `corrupt.journal~` | `compact-lz4` with its chain's first array and 40 arena bytes overwritten |
