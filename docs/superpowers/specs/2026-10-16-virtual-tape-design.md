# Virtual tape support for WMS backup and restore

## Intent and scope

Provide one file-backed tape drive so the original WMS backup and restore code
can operate through its own controller driver. The recovered SAVE application
invokes DSAVE for its midday and daily backup paths. A copied-state native run
now confirms DSAVE reaches System Save and selects Tape Unit 1.

The first version supports one mounted virtual reel on WMS Tape Unit 1. Create
new writable tape images for backup and mount existing images read-only for
restore. Success requires guest-generated tape records and a verified restore
onto a separate disposable disk state. A host-side archive of the disk is not
proof that the guest tape driver or DSAVE worked.

Preserve the immutable base disk, disk-overlay format, retained-RAM contract,
and all existing guest behavior when no tape is attached. Use disposable copies
for all private-image investigation and acceptance tests. Keep original images,
RAM snapshots, credentials, backup contents, and private traces out of Git.

## Investigation evidence

The original DSAVE object is a short stored procedure rather than a native CPU
program. After normal guest authentication, Advanced Processing provides
Procedure Control and System Functions. Executing DSAVE through Procedure
Control and accepting the guest confirmation reaches the same Tape Unit menu
as System Functions, System Save. The procedure selects Tape Unit 1 itself.

The unmodified Rust execution stops at guest word address `0xBD37` on instruction
`0x7A90`, decoded as `DOAC AC3, device 0o20`, with AC3 equal to `0x1000`.
Device numbers here are octal; PC values and register values are hexadecimal.
The current emulator implements only limited absent-device probes for this port.
This is the concrete first missing controller operation.

Normal boot also accesses device 0o20 using DOAS with `0x0810`, SKPS, and DIAC.
A research-only continuation that skips the unsupported output reaches DOA with
value zero at `0x23CD`, then DIA at `0xC2E1`. Skipping outputs is a tracing
hypothesis, not a recovered controller implementation or a successful backup.
Static driver code also contains self-modified I/O templates, so searching only
for literal device-0o20 opcodes does not enumerate the complete interface.

The status-check routine uses the low bit of the DIA result before choosing the
original TAPE UNIT NOT READY message. Another branch reads DIC status. Busy and
completion polling, data transfer, command meanings, density selection, and
interrupt acknowledgement must be recovered from the corresponding call sites
before presenting a writable drive as ready.

Private reproduction helpers and traces are under `/tmp/rocc-tape-investigation`.
SHA-256 checks confirmed the original disk, overlay, and RAM sidecar were
unchanged during investigation.

## Approach and alternatives

Implement a bounded tape-medium layer and a separate ROCC controller state
machine. The medium handles records and tape position. The controller handles
only proven register transfers, status bits, pulses, and completion behavior.
Continue decoding the guest driver as part of implementing that contract, and
keep unknown operations explicit failures until their behavior is established.

SIMH's standard tape-file representation is suitable for the medium and has an
existing read-only Python inspector in this workspace. A standard Nova tape
controller is a reference for comparison, not an assumed replacement: its
register values and device assignment are not established for this ROCC system.
A host disk-copy command would provide a separate backup utility but would not
satisfy guest tape emulation or validate DSAVE.

Primary references:

- https://simh.trailing-edge.com/docs/simh_magtape.pdf
- https://github.com/simh/simh/blob/master/NOVA/nova_mta.c
- Existing read-only container implementation: `rocc-analysis/rocc_tools/tape.py`
  in the parent workspace; it is not a Rust controller implementation.

## Media contract

Use the standard SIMH `.tap` subset: little-endian 32-bit leading record length,
record bytes, a padding byte for odd lengths, and the matching trailing length.
A tape mark is one zero 32-bit word. Recognize end-of-medium markers and physical
EOF. Preserve record boundaries and tape marks; a tape file is not a flat byte
stream. Reject malformed framing, unsupported object classes, invalid lengths,
and truncated records rather than silently treating them as valid tape data.

Cap tape files at 512 MiB, individual records at `0xFFFFFF` bytes, and parsed
objects at 100,000, matching the existing inspector's resource ceilings. These
are emulator limits, not historical tape capacity measurements. Enforce limits
before allocation or guest-visible completion. Write canonical zero padding for
new odd-length records; accept arbitrary padding on valid input records.

The medium tracks beginning of tape, current record position, tape marks, and
end of medium. Provide read, write, rewind, write tape mark, and forward/reverse
spacing as required by traced guest commands. Reading or spacing beyond the
medium returns a defined controller status and never synthesizes a data record.
A read-only mount reports write protection and refuses all modifying commands.

## Controller contract and evidence gate

Add a dedicated controller module rather than extending the generic absent
port branch with unconditional successes. It receives checked memory access and
its I/O operation, pulse, and selected accumulator value. It owns its command,
status, transfer progress, selected unit, busy/done state, and interrupt state.

For each implemented operation, retain a provenance table linking its command
encoding, observed guest call site, expected status/polling, and the test that
exercises the behavior. Cover startup discovery as well as backup and restore.
Do not enable ready/present responses until the supported command sequence has
working status, transfer, and completion semantics. Unknown commands, pulse
combinations, invalid unit encodings, or transfer modes remain explicit typed
faults. Valid selections of unconfigured units report unavailable.

Determine the exact data-transfer convention from the original driver. If it is
DMA, validate the whole guest address range and mapping before a transfer; if it
is programmed I/O, bound the staging buffer and account for each handshake.
Never choose DMA or programmed I/O merely from the generic Nova reference.
Preserve big-endian guest word ordering when forming tape record bytes. Resolve
odd-byte counts and short records explicitly through status and residual counts.

Expose tape completion through the existing interrupt arbitration only after
recovering its priority, mask, and acknowledgement behavior. Controller reset
clears controller state without erasing tape contents. A mounted medium starts
at beginning of tape; mounting on a new emulator launch represents a newly
loaded reel, not an instruction-level checkpoint of a prior tape operation.

## CLI and file handling

Add mutually exclusive `--tape PATH` and `--tape-create NEWPATH` options to boot.
`--tape` mounts an existing image read-only; `--tape-create` creates a new writable
image without overwriting any existing path. Both require the extended profile.
Without either option, preserve the existing absent-device behavior. Tape Unit 1
is the only configured drive; other units must report unavailable.

Validate and open tape media before marking RAM active or executing any guest
instructions. Reject image/overlay/RAM/report aliases, final symlinks, nonregular
files, unsupported formats, incompatible flags, and resource-limit violations.
Use private permissions for new tape files and hold an exclusive advisory lock
on a writable reel for the mounted session. Read-only tape input is never changed.

Flush complete framed records and tape marks before acknowledging guest-visible
write completion. Sequential appends must not rewrite the entire accumulated
backup for every record. If a proven guest operation overwrites earlier tape,
replace the suffix with defined record-boundary semantics and preserve the
original mounted read-only input. A write failure must stop or return a proven
error status, never successful completion. An interrupted new tape may be an
incomplete backup and must not be reported as restorable.

Keep tape persistence separate from the RAM snapshot format. RAM restore still
boots the guest with reset controllers, and a tape mounted on that launch begins
at BOT. Do not claim that RAM persistence resumes in-flight tape operations.

## Diagnostics

Report whether a tape was mounted, read-only versus new writable mode, selected
unit, initial media identity, final committed media identity, record and tape-mark
counts, current position, read/write totals, and controller stop/error state.
Capture bounded recent controller events sufficient to debug command sequences;
do not include tape record payloads in ordinary diagnostic reports.

Document the exact supported WMS backup and restore routes, the single-drive
limit, mounting behavior, file-format subset, and durability boundaries. Maintain
an explicit distinction between completed backup, successful structural parsing,
and successful restore verification.

## Verification and acceptance

Use failing synthetic tests before implementing the medium and each controller
behavior. Cover even/odd data records and zero-length tape marks, multiple files
separated by tape marks, BOT/EOM, short reads, backspacing, rewind, write
protection, capacity limits, invalid framing, and persistence failures. Inspect
Rust-generated tapes independently with the existing Python parser.

Controller tests must cover every recovered register/pulse combination, busy and
done transitions, reset, interrupts and masks, transfer bounds, byte ordering,
and rejection of unknown operations. Guest-memory faults must precede partial
DMA effects. Retain absent-device compatibility when the drive is not mounted.

On copied private disk/RAM state, run original DSAVE and confirm the generated
tape has records and tape marks consistent with the guest's completion path.
Then mount it read-only and execute the original restore path against a separate
disposable destination. Compare the backed-up system/library/file contents and
metadata that the selected WMS operation promises to save; do not require raw
unused disk sectors or unrelated runtime timestamps to match. Demonstrate that
restored contents are not merely inherited from an untouched clone of the source.
Verify the restored system can boot when the selected save operation includes
system recovery. Do not claim full-disk disaster recovery from a file-only save.

Verify original disk, overlay, RAM, and input tape hashes remain unchanged. Run
Rust tests, CLI tests, frozen and independent CPU regressions, formatting, Clippy,
and release compilation. Complete an independent review before reporting tape
backup and restore as supported. If a hardware boundary remains unresolved,
report that boundary instead of presenting a partial trace as a working backup.
