#!/bin/sh
# Recreates tests/fixtures/generated/ and tests/oracle/.
#
# Every entry is synthetic, written below in the journal export format
# (https://systemd.io/JOURNAL_EXPORT_FORMATS/) and turned into journal files
# by systemd-journal-remote; no real log goes in. journalctl's export of
# each file is the oracle tests/oracle.rs compares against.
#
# Run on a Linux machine with systemd (made with systemd 257 on Debian 13):
#   sudo apt-get install systemd-journal-remote
#   sh tests/fixtures/generate.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
out="$here/generated"
oracle="$here/../oracle"
remote=/lib/systemd/systemd-journal-remote
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$out" "$oracle"

python3 - "$work/export" <<'EOF'
import struct, sys

MACHINE = "5d0c7f4a0b8e4c51a3a1b7e0c2d9f611"
BOOTS = ["8f1e2d3c4b5a69788796a5b4c3d2e1f0", "0a1b2c3d4e5f60718293a4b5c6d7e8f9"]
BASE = 1_700_000_000_000_000  # 2023-11-14T22:13:20Z, in microseconds

def field(name, value):
    if isinstance(value, bytes):
        return name.encode() + b"\n" + struct.pack("<Q", len(value)) + value + b"\n"
    return f"{name}={value}\n".encode()

def entry(boot, n, message, ident, pid, comm, exe, cmdline, unit, uid=0,
          priority=6, transport="syslog", extra=()):
    when = BASE + boot * 3_600_000_000 + n * 1_250_000 + 17
    fields = [
        ("__REALTIME_TIMESTAMP", when),
        ("__MONOTONIC_TIMESTAMP", 4_000_000 + n * 1_250_000),
        ("_BOOT_ID", BOOTS[boot]),
        ("_MACHINE_ID", MACHINE),
        ("_HOSTNAME", "lab-host"),
        ("_TRANSPORT", transport),
        ("PRIORITY", priority),
        ("SYSLOG_FACILITY", 4 if ident in ("sshd", "sudo") else 3),
        ("SYSLOG_IDENTIFIER", ident),
        ("_PID", pid),
        ("_UID", uid),
        ("_GID", uid),
        ("_COMM", comm),
        ("_EXE", exe),
        ("_CMDLINE", cmdline),
        ("_SYSTEMD_UNIT", unit),
        ("_SOURCE_REALTIME_TIMESTAMP", when - 5),
        ("MESSAGE", message),
        *extra,
    ]
    return b"".join(field(k, v) for k, v in fields) + b"\n"

sshd = dict(ident="sshd", comm="sshd", exe="/usr/sbin/sshd",
            cmdline="sshd: alice [priv]", unit="ssh.service")
sudo = dict(ident="sudo", comm="sudo", exe="/usr/bin/sudo",
            cmdline="sudo systemctl restart nginx", unit="session-3.scope", uid=1000)
long_text = "".join(f"line {i:03}: backup of /srv/data/part-{i:03} finished, 4096 blocks copied; "
                    for i in range(12))
binary = bytes(range(256)) * 2 + b"\x00\n=binary tail\xff"

entries = []
for boot in (0, 1):
    n = 0
    def add(message, **kw):
        global n
        entries.append(entry(boot, n, message, **kw))
        n += 1
    add("Linux version 6.1.0-lab (synthetic) #1 SMP", ident="kernel", pid=0,
        comm="kernel", exe="/boot/vmlinuz", cmdline="BOOT_IMAGE=/vmlinuz",
        unit="init.scope", transport="kernel", priority=5)
    add("Server listening on 0.0.0.0 port 22.", pid=611 + boot, **sshd)
    add("Failed password for invalid user admin from 203.0.113.7 port 50122 ssh2",
        pid=1201 + boot, priority=5, **sshd)
    add("Failed password for root from 203.0.113.7 port 50130 ssh2",
        pid=1202 + boot, priority=5, **sshd)
    add("Accepted publickey for alice from 198.51.100.23 port 51514 ssh2: "
        "ED25519 SHA256:2b5Zr4M3p0VqfYpGx5gO7q1sL9cT8dWb0eH3kJ6nA2Y",
        pid=1210 + boot, **sshd)
    add("pam_unix(sshd:session): session opened for user alice(uid=1000) by alice(uid=0)",
        pid=1210 + boot, **sshd)
    add("alice : TTY=pts/0 ; PWD=/home/alice ; USER=root ; COMMAND=/usr/bin/systemctl restart nginx",
        pid=1290 + boot, priority=5, **sudo)
    add("pam_unix(sudo:session): session opened for user root(uid=0) by alice(uid=1000)",
        pid=1290 + boot, **sudo)
    add(long_text, ident="backup", pid=1400 + boot, comm="backup.sh",
        exe="/usr/bin/bash", cmdline="/bin/bash /usr/local/bin/backup.sh",
        unit="backup.service", transport="stdout")
    add("core dump attached", ident="crasher", pid=1500 + boot, comm="crasher",
        exe="/usr/local/bin/crasher", cmdline="/usr/local/bin/crasher --once",
        unit="crasher.service", priority=2, transport="journal",
        extra=[("PAYLOAD", binary), ("CODE_LINE", 42)])
    add("Received disconnect from 198.51.100.23 port 51514:11: disconnected by user",
        pid=1210 + boot, **sshd)
    add("pam_unix(sshd:session): session closed for user alice", pid=1210 + boot, **sshd)

with open(sys.argv[1], "wb") as f:
    f.write(b"".join(entries))
EOF

# journal-remote stamps its own machine's id in the file header: show it a
# synthetic one, in a private mount namespace.
echo 5d0c7f4a0b8e4c51a3a1b7e0c2d9f611 >"$work/machine-id"
make() { # name, then environment
    name=$1
    shift
    sudo unshare --mount sh -c 'mount --bind "$1" /etc/machine-id && shift && exec "$@"' \
        sh "$work/machine-id" env "$@" "$remote" --output="$work/$name.journal" - <"$work/export"
    sudo chown "$(id -u):$(id -g)" "$work/$name.journal"
    journalctl --file="$work/$name.journal" -o export --all >"$oracle/$name.export"
}

# A file is allocated in steps up to its maximum size: make that maximum
# small (512 KiB). The unused tail is zeros, which gzip removes.
conf=/etc/systemd/journal-remote.conf.d/zz-sootmark-fixtures.conf
sudo mkdir -p "$(dirname "$conf")"
printf '[Remote]\nMaxFileSize=512K\n' | sudo tee "$conf" >/dev/null
trap 'sudo rm -f "$conf"; rm -rf "$work"' EXIT

make regular SYSTEMD_JOURNAL_COMPACT=0 SYSTEMD_JOURNAL_COMPRESS=NONE
make compact SYSTEMD_JOURNAL_COMPACT=1 SYSTEMD_JOURNAL_COMPRESS=NONE
make lz4 SYSTEMD_JOURNAL_COMPACT=0 SYSTEMD_JOURNAL_COMPRESS=LZ4
make compact-lz4 SYSTEMD_JOURNAL_COMPACT=1 SYSTEMD_JOURNAL_COMPRESS=LZ4
make zstd SYSTEMD_JOURNAL_COMPACT=1 SYSTEMD_JOURNAL_COMPRESS=ZSTD
make xz SYSTEMD_JOURNAL_COMPACT=0 SYSTEMD_JOURNAL_COMPRESS=XZ
make unkeyed SYSTEMD_JOURNAL_COMPACT=0 SYSTEMD_JOURNAL_KEYED_HASH=0 SYSTEMD_JOURNAL_COMPRESS=NONE

# Damaged copies, the way evidence arrives:
# - online: copied while journald had it open (state ONLINE);
# - truncated: cut two thirds of the way into its objects;
# - corrupt: the entry array chain's first array and a stretch of the arena
#   overwritten, as by a bad sector; named the way journald renames a file
#   it finds damaged.
python3 - "$work" <<'EOF'
import struct, sys

work = sys.argv[1]

def read(name):
    with open(f"{work}/{name}", "rb") as f:
        return bytearray(f.read())

def write(name, data):
    with open(f"{work}/{name}", "wb") as f:
        f.write(data)

def u64(data, offset):
    return struct.unpack_from("<Q", data, offset)[0]

def objects(data):
    """Where the objects after the hash tables start, and the last one."""
    tables_end = max(u64(data, 104) + u64(data, 112), u64(data, 120) + u64(data, 128))
    return tables_end, u64(data, 136)  # tail_object_offset

online = read("compact.journal")
online[16] = 1  # state
write("online.journal", online)

regular = read("regular.journal")
start, tail = objects(regular)
write("truncated.journal", regular[: start + (tail - start) * 2 // 3 + 3])

corrupt = read("compact-lz4.journal")
chain = u64(corrupt, 176)  # entry_array_offset
corrupt[chain + 8 : chain + 16] = b"\xff" * 8  # the array's object size
start, tail = objects(corrupt)
middle = (start + tail) // 2 // 8 * 8
corrupt[middle : middle + 40] = b"\xa5" * 40
write("corrupt.journal~", corrupt)
EOF

for file in "$work"/*.journal "$work"/*.journal~; do
    gzip -9 -n -c "$file" >"$out/$(basename "$file").gz"
done
