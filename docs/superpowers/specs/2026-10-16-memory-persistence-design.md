# Memory persistence for WMS restart

## Intent and approved scope

Preserve guest RAM alongside the persistent disk overlay so successive emulator
launches can run the WMS restart path. Normal startup restores retained RAM and
executes the bootstrap with reset CPU and controller state. This models retained
memory rather than resuming an instruction-level machine checkpoint.

The approved first version saves on orderly stops and signals handled by the
interactive loop. Forced termination, process crashes, and host power failure
do not guarantee a new RAM save. Detect an interrupted session and require
explicit cold recovery instead of silently restoring an older memory snapshot.
The base image remains read-only, and the ROCCOVL1 disk format remains unchanged.

## Evidence and restart behavior

With a previously used Fujitsu overlay and newly allocated unknown RAM, WMS
enters a loader branch that reads front-panel switches, then accesses retained
configuration at RAM word 411. A zero switch value alone does not provide that
configuration. The existing `--cold-boot` option follows the loader's operator
recovery sequence and initializes memory through guest instructions.

A local diagnostic run using a copy of the overlay reached the main menu after
27 million instructions with cold recovery. A second machine retained all
physical memory from that run, reset the active mapping to bank zero, reloaded
the effective bootstrap block at words 0 through 127, and started with the
existing reset CPU and controller defaults. It reached the WMS4 Main Menu after
27 million instructions without cold recovery or a guest fault. This establishes
the restart behavior for the available image; it does not establish every
physical reset detail or other WMS versions.

## Selected approach and alternatives

Store RAM in a versioned binary sidecar derived from the selected disk overlay.
This extends the existing boot lifecycle while keeping memory serialization and
file handling separate from instruction execution.

A complete machine checkpoint would also preserve registers, pending controller
events, instruction timing, and display state. That supports execution resume,
but is unnecessary for the demonstrated bootstrap restart. Continuously durable
RAM would require a journal or another consistency mechanism coordinated with
disk writes; it is outside the approved orderly-exit scope.

## User behavior

Every CLI boot uses memory persistence by default. If the selected overlay is
`disk.img.overlay`, its memory sidecar is `disk.img.overlay.ram`. An explicit
`--overlay` therefore selects a separate disk and memory pair. Derive the sidecar
from the canonical overlay path after acquiring the overlay lock.

When no memory sidecar exists, start with the existing bootstrap initialization.
An existing WMS overlay without its original RAM still needs `--cold-boot` once.
After a successful cold session saves RAM, subsequent launches use the ordinary
boot command. A fresh overlay remains usable without a special first-run flag.

`--cold-boot` deliberately bypasses RAM restoration and retains the existing
verified panel sequence. On a successful orderly stop, replace the previous
sidecar with the newly initialized RAM. This operation does not erase disk data;
the guest performs its own initialization and disk writes.

Add `--no-memory` to disable both RAM loading and RAM saving for an explicitly
stateless run. Such a run may make an existing memory snapshot incompatible by
changing the disk overlay. The next persistent run must detect that mismatch.
Do not add an independent memory filename override in this version.

## Ownership and interfaces

`Memory` owns conversion between its private physical storage and a checked
snapshot representation. Preserve the exact distinction between unknown words
and initialized zero words. Import validates geometry and the entire payload
before replacing any live memory. Never expose mutable storage directly.

A dedicated persistence module owns the sidecar descriptor, format validation,
session marker, hashing, and atomic publication. `Machine` owns applying retained
memory and bootstrap initialization. CLI orchestration acquires the disk overlay,
validates artifact destinations, attaches memory persistence before the first
guest instruction, and finalizes it when execution returns an orderly outcome.

Keep persistence out of `Machine::step`, CPU execution, DMA, and the terminal
event loop. Existing library construction and `cpu-cases` remain filesystem-free;
the CLI explicitly attaches the persistence lifecycle. Library callers can opt
into the same attachment and finalization APIs.

## Startup ordering

1. Parse options and validate the machine configuration and report destination.
2. Read the immutable base image and acquire the existing exclusive disk-overlay
   lock. Keep that lock through memory finalization.
3. Derive and validate the memory path. Reject report, base-image, disk-overlay,
   or other protected-file aliases before mutating the sidecar.
4. Unless `--cold-boot` or `--no-memory` was selected, load and validate the saved
   memory against the selected image, effective disk-overlay state, and memory
   geometry. Do not run guest instructions if validation fails.
5. Apply validated RAM, reset mapping to bank zero, and overwrite words 0 through
   127 with the effective disk bootstrap block, including overlay block zero if
   present. Preserve saved word 10941; its existing zero initialization applies
   only when starting without retained memory. CPU registers, controller state,
   terminal state, and instruction counters use their current reset defaults.
6. Durably mark the sidecar as an active, uncommitted session before executing
   the first guest instruction. A new sidecar may initially contain only a
   recognized uncommitted marker. `--no-memory` skips this step entirely.

With an uncommitted marker, normal startup stops with an actionable message
directing the user to `--cold-boot`. Cold recovery may replace a recognized
sidecar even if its saved payload is damaged or incompatible. An arbitrary file
without the sidecar's format identifier must never be overwritten automatically.

## Snapshot format and validation

Use a fixed binary header with a format identifier and version, committed versus
active status, CPU profile and address width, physical memory geometry, base
image length and SHA-256, effective overlay-state SHA-256, and a checksum covering
the committed header and payload. No runtime-dependent Rust struct layout or
native-endian encoding is part of the file format.

Encode each physical word as an initialized flag plus a big-endian 16-bit value.
An unknown word has a zero flag and canonical zero value; a known zero has a set
flag and zero value. Serialize every allocated bank, including banks not mapped
at exit. The largest current configuration has 585,728 physical words, with a
1,757,184-byte payload. Validate counts against the configured geometry before
allocating, and bound total reads to that payload plus the fixed header.

Reject unsupported versions, invalid flags or geometry, noncanonical unknown
words, truncated payloads, trailing bytes, and checksum failures. Ordinary
restore also rejects an active-session marker, a different base image, or a
different effective overlay digest. The existing sorted changed-block digest
allows a compacted but logically identical disk overlay to match its RAM.

Terminal settings, capture limits, connection timing, and batch versus interactive
mode are not memory geometry and do not invalidate a retained-memory snapshot.
No CPU registers, bank selector, device queues, guest step counter, or screen
contents are restored.

## Save and failure behavior

Save after user exit, HALT, a completed instruction budget, input EOF, or a signal
handled by the interactive loop. Terminal restoration occurs through the existing
cleanup path before potentially slow snapshot publication. Preserve signal exit
codes when saving succeeds. Unhandled batch signals are outside this guarantee.

Publish RAM before writing an optional diagnostic report, so report publication
failure does not discard an otherwise completed RAM save. Snapshot finalization
failure produces a nonzero result with a diagnostic identifying the memory
path. Failure before replacement leaves the active marker; failure after atomic
replacement can leave the complete new snapshot, which remains subject to normal
validation on the next launch. Never report a successful durable memory save
unless publication and directory synchronization succeeded.

Do not commit a snapshot after a guest fault, a failed disk-overlay write, an
interactive I/O error, or an internal execution error. Leave the active marker
so the next launch cannot silently treat that session as a clean shutdown.
The disk overlay retains its existing independent record-recovery behavior.

The active marker is essential even when the guest did not write to disk:
otherwise a forced kill could leave an old snapshot with a matching overlay
digest and incorrectly appear to be a clean restart. A crash between complete
snapshot publication and final directory synchronization may leave a usable new
snapshot or require cold recovery; no host power-loss durability is promised.

## File protection and concurrency

Create sidecars and publication temporaries privately with mode 0600. Refuse
symlinks, nonregular files, and multiply linked sidecars before any in-place
marker update. Check file identities against the open base image and overlay,
and reject a report destination that aliases the sidecar. Perform validation
through open descriptors using no-follow and nonblocking flags where available.

The existing disk-overlay lock serializes the derived memory sidecar lifecycle.
An independent memory path is not exposed, so a separate lock namespace is not
needed. Preserve an already existing memory file until it has been recognized
as emulator-owned and the run's destinations have passed validation.

For committed saves, write a complete new snapshot to a unique temporary file
in the same directory, sync it, atomically replace the recognized sidecar, and
sync the directory. Do not change the existing no-overwrite publication behavior
for reports or compacted disk overlays. Clean up unpublished temporary files on
failure; never alter the base image or another runtime artifact as cleanup.

## Diagnostics and documentation

Report whether memory persistence was enabled, the sidecar path, whether RAM was
restored, whether it was bypassed for cold boot, and whether the final snapshot
was committed. Record the loaded snapshot identity when present. Keep the existing
final memory digest and initial and final overlay digests.

Add a concise boot-summary line showing RAM restored or initialized and saved,
or disabled. Errors distinguish corrupted state, interrupted prior session,
incompatible geometry, mismatched disk state, and publication failure. Document
the ordinary boot workflow, one-time recovery for existing disk-only overlays,
`--no-memory`, sidecar pairing, and the orderly-exit durability limit.

## Verification and acceptance

Add failing regression tests before implementation for physical-memory round
trip, all banks, initialized zero versus unknown words, bootstrap overwrite,
preserved word 10941, geometry and image mismatches, overlay-state mismatches,
corrupt or truncated files, and interrupted-session markers.

CLI tests must exercise automatic save and restore across separate processes,
custom overlay pairing, stateless opt-out, cold recovery, guest-fault handling,
save failure, path aliases, symlinks, permissions, and concurrent overlay use.
Use synthetic images to establish saved RAM affects the next boot. Verify
handled interactive signals save memory and restore terminal state. Kill a child
process after its active marker is durable and assert that normal restoration
refuses it rather than promising crash recovery.

On copies of the private Fujitsu overlay, first cold boot and save RAM, then
launch a new process without `--cold-boot` and confirm the WMS4 Main Menu. Repeat
the normal restart and exercise interactive exit. Hash the original image and
overlay before and after to establish that validation did not change them.
Private images, memory snapshots, and runtime reports stay outside the repository.

Run the Rust tests, CLI tests, frozen and independent CPU regressions, formatting,
Clippy, and release build. Have the final change reviewed before completion.
