# ROCC tape controller evidence

Device numbers are octal; instruction addresses and register values are hexadecimal.
This implementation targets the **polled WMS paths** at device `0o20`.
It does not claim a complete physical-controller model. Interrupt-driven commands
and untraced mode bits remain unsupported.

## Recovered interface

| Operation | Driver evidence | Contract |
| --- | --- | --- |
| DOAC `1000` | `BD35..BD37` | Initial controller clear; no data transfer. |
| DOAS `0810`, DIAC | Startup `F08A..F08E`; offline `09AC..09C0` | Controller-mode probe followed by status/clear. |
| DOA | `23B9..23CD`, `0CE6..0CEB` | Select command and unit without starting a transfer. |
| DIA | `C2D6..C2F1`, `0CC9..0CE4`, `0BC4..0BC9` | Bit `0001` ready; `0100` hard error; `0200` EOF; `2000` rewind state. |
| DOB | `697A..697C`, `6A00..6A01`, `0CF1..0CF3` | Set DMA word address. |
| DOCS | `697D..6981`, `6A04..6A07`, `0CF4..0CF8` | Negative transfer word count; start the selected operation. |
| DIB | `6A16..6A25`, `0970..099B`, `0C19..0C26` | DMA end word address, with the driver's address-width masking. |
| NIOC and SKPS | `2389..239E`, `0C63..0C74` | Clear controller completion; wait until not busy. |

The first native DSAVE write sequence is DOA `0028`, DOB `0847`, DOCS
`7F80`. The buffer is a 128-word bootstrap record, beginning with executable
instructions, not a flat host disk copy. Its prefix is not reproduced here.
The offline driver then writes memory pages and disk records. Generic library
read/write routines use the same register interface through self-modified I/O.

The opcode fields also agree with the primary
[Data General peripheral programming manual](https://www.bitsavers.org/pdf/dg/015-000021-07_PeriphPgm_Aug77.pdf)
and [SIMH Nova tape implementation](https://github.com/simh/simh/blob/master/NOVA/nova_mta.c):
read `00`, rewind `08`, controller mode `10`, space forward `18`, space reverse
`20`, write `28`, tape mark `30`, erase gap `38`; unit is the low three bits.
WMS Tape Unit 1 selects physical unit zero. The ROCC port assignment is recovered
independently; it is not the standard Nova MTA port.

The driver checks write protection `0004`, BOT `0080`, EOF `0200`, and read/write
error masks `5104`/`5504`. Standard status also identifies nine-track `0040` and
high density `0800`. Count inputs used by the recovered paths are negative
15-bit values (sometimes sign-extended to 16 bits); supported data transfers
are limited to 1..4096 words. The original system loader also issues count zero
to read a whole record; this mode remains bounded to 4096 words. Larger/ambiguous
hardware counter modes fault.
Odd input records pad the final guest word with zero and report odd length;
oversized records report count overflow rather than overwriting following RAM.

ROCC EOF is not the stock Nova file-mark bit. Native restore showed that `0100`
enters the immediate error path at `0CDC..0CE4`; `0200` is recorded as EOF at
`0CD3..0CD5` and handled separately by the reader. Successful mark writes leave
EOF clear: `0CB7..0CBF` otherwise retries a completed mark as an error. Native
backup and restore validate these distinctions. EOM is modeled as EOF; physical
reel distance and early-warning EOT are not calibrated.

## Completion and reset

The supported WMS routines poll busy with SKPS and then read status; they do not
rely on a tape interrupt to complete these operations. A start sets busy and
clears done; the next device tick performs the checked operation, then clears
busy and sets done only after durable writes. Clear/reset cancels pending work
without erasing the medium or rewinding an existing mounted reel. New mounts
begin at BOT. No tape interrupt is emitted until its ROCC mask/priority contract
is independently recovered; mode bits requesting unsupported polling/interrupt
facilities are rejected, rather than inheriting a stock Nova interrupt map.

## Dependencies and validation status

Native full-disk save switches to the offline driver. Investigation exposed disk
control `2800` at `0558`, disk control `2000/C=0C00` at `0277`, NIOP at `027A`,
compound writes with `3000` command bits, and mask `FFBF` enabling disk completion
through bit `0040`. The existing disk model's mask `2000` is provisional and
does not permit that completion. The extended profile now uses bit `0040`,
supports these exact controls and compound writes, and allows auxiliary clear
pulses to restart initialization when the guest returns from offline save.

Disposable research continuations supplied status/accepted outputs to reach the
DMA call sites. They establish the driver's requests and branch conditions,
not successful backups or hardware captures. No private trace, payload,
credential, or disk image belongs in the public fixture. Acceptance uses native
runs without these research substitutions.

## Assisted operator IPL

`examples/wms_tape_restore.rs` supplies the operator boot-ROM boundary for this
particular saved layout. It requires an all-zero disk base, a new overlay and a
read-only reel. It clears physical RAM (including unused banks), validates the
128-word bootstrap signature, reads the first 4096-word RAM record through the
same controller, validates the original busy-poll entry at word `0867`, then
transfers control there. Subsequent memory loads, bank selection, tape reads,
disk writes and restart instructions are original guest code. No guest
instruction is patched. Without memory clear, the restored kernel can read
unknown cells in an unused bank while rebuilding its cache. This helper is not
an implementation of an unidentified physical ROM.

The main `boot` command continues to boot a disk. The example supplies a separate
tape-load entry without changing disk/RAM startup semantics.

## Native acceptance

On disposable private state, unmodified DSAVE selected Tape Unit 1, wrote 11,498
data records and three marks (92,774,968 bytes), rewound and returned to the WMS
menu without a fault. The independent Python SIMH reader parsed the complete
reel. Original disk, overlay and retained-RAM SHA-256 values remained unchanged.

The native system loader read every tape object and restored a blank disk through
a separate overlay. The restored disk cold-booted to WMS4 Main Menu. Logical
payload/source comparison covers all 1,420 catalogue objects; point-in-time
journal comparison is distinguished from post-restart journal activity.
Acceptance artifacts, detailed inventories and credentials remain outside Git.
