# Emulator hardening and architecture design

## Intent and scope

The user requested implementation of every finding and architectural recommendation
from the emulator review. Success means bounded runtime resources, reliable host
terminal cleanup, private and durable exports, enforced memory/configuration
invariants, narrower controller interfaces, maintainable overlays, reproducible
reports, and automated validation. Preserve recovered guest instruction behavior,
interrupt ordering, existing boot CLI defaults, and the ROCCOVL1 overlay format.
No base image may be modified. Public Rust interfaces may change to enforce
invariants; migrate all repository callers and document those changes.

This document is the architectural design for review. It does not claim that
implementation has begun or that the findings have been fixed.

## Approach and alternatives

Use incremental changes with regression checks between boundary changes. Begin
with regression tests for the confirmed failures, then introduce resource and
lifecycle fixes, architecture changes, reporting, overlay tooling, and CI.

Local patches alone would resolve immediate failures while retaining independently
mutable configuration and broad controller access. A replacement emulator would
discard useful validation and increase the risk of changing inferred semantics.
Neither is the selected approach.

## 1. Configuration, memory, and controller boundaries

Introduce a validated MachineConfig containing named CPU and terminal profiles,
address width, connection step, and diagnostic/event limits. Reject incompatible
profile/width combinations through Result, before allocating a machine or
creating runtime artifacts. Retain a compatibility Machine::new constructor if
it can delegate entirely to validation; configuration stored in a running machine
must not be independently mutable.

Memory owns storage geometry, active mapping, and global-only mode. Make geometry,
storage, and mapping private. Checked APIs distinguish ordinary CPU access,
selected-bank DMA ranges, explicit-bank copy-source access, and physical fixture
initialization. Use a validated mapping enum for SelectedBank and GlobalOnly.
An invalid address or selector returns a typed error rather than indexing or
assertion panic. Preserve unknown-word tracking, forward-copy overlap, explicit
source-bank behavior, and atomic rejected block copies and disk transfers.

Controllers receive Memory and their required transfer values rather than Cpu.
Bank-control operations change Memory's mapping; they cannot change accumulators
or PC. Cpu owns registers and instruction execution. Machine owns instruction
time, terminal ticking, deferred interrupt recognition, and execution accounting.
Separate these steps without changing the order: terminal tick; deferred-enable
update or IRQ entry; instruction fetch and execution; step accounting. The first
handler instruction still executes in the IRQ-entry step. Faulting steps still
advance the instruction clock once. Fixture execution must use the same stepping
path rather than maintaining a second interrupt/timing implementation.

Use typed fault kinds and contexts with stable serialized reason strings.
Replace JSON creation on successful controller paths with typed packet/event
records. Model terminal transfer readiness and disk control/transfer outcomes
with enums, retaining partially initialized device registers where the hardware
protocol permits them. Protocol operations must not depend on unwrap for a state
that can be absent. Serialization remains at the diagnostic/export boundary.

## 2. Bounded terminal resources

Default pending-event capacity is 4096. Connection, host-key, acknowledgement,
and timer events all pass through one checked queue API. Coalesce a pending timer
event using explicit state, updated when that event is removed, avoiding a scan.

Before any command changes controller state, reads a transfer, changes the screen,
or acknowledges completion, reserve any required event slot. If reservation fails,
return terminal_event_limit without partial effects. Host key delivery also
returns that fault rather than silently losing input. If an instruction-clock
event cannot be queued, stop before executing that instruction and report the
resource fault. This is an explicit emulator policy, not a hardware claim.

Bound display history independently to 1 MiB of bytes and 4096 packets in both
batch and interactive modes. Batch mode stops before a transfer that exceeds
either limit; retain its existing fail-before-capture policy. Interactive mode
evicts the oldest history while continuing to update the complete screen.
Use a byte ring buffer and compact typed packets. Define separate byte-history
and packet-history truncation flags, plus retained and total counts. Keep bounded
screen unknown/control histories and host keyboard/notices.

Make limits configurable through validated library configuration and boot options
for event capacity, capture bytes, and packet capacity. Reject zero limits and
values exceeding documented ceilings: 65536 events, 64 MiB capture bytes, and
65536 packets. Record effective limits in reports.

## 3. Host terminal lifecycle

Install scoped handlers for SIGINT, SIGTERM, and SIGHUP during interactive mode.
Handlers only set a signal-safe shutdown flag. The main loop observes the flag,
records termination, and exits through RawSession cleanup. Restore previous
handlers on every exit, including setup failures. No formatting, allocation, or
terminal I/O occurs in a signal handler. Return conventional signal exit status
(128 + signal number) after cleanup and any requested report publication.

Keyboard Ctrl-C and Ctrl-] retain their existing user_exit behavior. SIGKILL
cannot be handled and is excluded from cleanup guarantees. PTY tests must verify
termios restoration for normal exit, guest fault, each handled signal, and errors
after raw mode entry. Preserve alternate-screen and cursor restoration.

## 4. Private, durable, bounded report export

Create temporary report files with mode 0600 from the first write, regardless of
a permissive umask. Preserve no-overwrite publication and symlink protection.
Use collision-resistant create_new temporary names; clean up on errors. Sync
file content before publication and sync the parent directory after publication
and temporary-file removal. A publication failure must preserve an existing
destination and leave the emulator/base image state intact.

Provide a lightweight RunSummary for ordinary CLI output. Do not hash memory,
encode modified disk blocks, clone packet history, or build a full report when
--report is absent. Preserve the displayed final screen through Screen accessors.

Serialize detailed reports from borrowed structures and iterators. Stream memory
hash input and changed-block encoding using bounded scratch space rather than
constructing a duplicate JSON tree or serialized-memory vector. Existing report
fields and stable reason strings remain available for the harnesses.

Add report_schema_version, package version, build revision (or an explicit
unavailable value), effective configuration, address width, instruction budget,
connection step, and batch key schedule. Record delivered interactive keys with
their instruction steps in a bounded replay history; expose an explicit truncation
flag and whether full replay is possible. Reproducibility metadata must not create
another unbounded buffer.

Record a digest of initial overlay state independently of the immutable base-image
digest. Distinguish final overlay state. Define display digests by scope: a full
stream digest updated incrementally and a retained-byte-history digest. Preserve
the legacy display sha256 field as the retained-history digest and document it.
Reports describe a run; they are not complete machine checkpoints.

## 5. Explicit overlay compaction

Add `compact-overlay IMAGE --overlay SOURCE --output NEWPATH`. Require a new
output path; never compact in place or overwrite the base/source. Hold the source
overlay's existing exclusive lock for validation and the complete snapshot/export
operation. Validate the original image and every source record before creating
the destination. Reuse the existing format and checksum implementation.

Write one record per latest changed block in deterministic offset order. Publish
the compacted journal using the same private, durable, no-overwrite file helper
as reports. The new overlay can subsequently be selected with --overlay. Leaving
the original journal untouched avoids a rename/file-lock inode race and gives
the user an immediate recovery copy. Existing boot commands and overlays remain
compatible. Report original size, compacted size, and retained block count.

Test source contention, corruption, wrong base, alias paths, existing destinations,
partial writes/publication errors, and boot equivalence between original and
compacted journals. No merge-to-base operation is added.

## 6. Validation, provenance, and automation

Keep all 8262 existing frozen-reference cases and their checksum unchanged.
The repository only identifies their source as synthetic frozen reference
results with a seed; do not invent missing oracle provenance or claim to recover
the original generator. Document this limitation explicitly.

Add a deterministic, separately implemented generator/reference model for a new
synthetic corpus, including its seed, model version, exact regeneration command,
and provenance. Do not calculate expected results by invoking the emulator under
test. Include arithmetic and addressing properties plus short integration
programs whose expected state transitions are independently specified.

Add multi-instruction programs combining deferred interrupts, masking, bank
switching, disk DMA, and terminal acknowledgements. Cover zero/exact-end/out-of-
bounds DMA ranges, global/local crossings, uninitialized source words, failed
transfer atomicity, saturated queues, and terminal command state transitions.
Add permission, signal, report-metadata/streaming, compaction, and injected
persistence failure regressions. Update Python harnesses with subprocess timeouts
so regressions fail rather than hanging CI.

Add GitHub Actions on pushes and pull requests. Test Rust 1.87 and stable on
Ubuntu using locked dependencies, cargo test, a release build, and both Python
harnesses. Run formatting and Clippy with warnings denied on stable. If current
locked dependencies cannot build on 1.87, select compatible dependency versions
and update Cargo.lock rather than silently increasing the declared minimum.
Use synthetic temporary images only; no private image or credentials enter CI.

Add repeatable performance probes for instruction throughput, sustained output
after history fills, large report export, and replay before/after compaction.
Record baselines and measurements; avoid machine-dependent timing assertions in
correctness tests. Update README, architecture/ISA documentation, CLI help, and
source-layout guidance to match the final implementation.

## Acceptance checklist

- Event saturation has deterministic faults and no partial command effects.
- Both capture modes enforce packet/byte limits; live retention has no repeated
  whole-history front shifts and timer coalescing has no queue scan.
- SIGINT, SIGTERM, and SIGHUP restore PTY termios before process exit.
- Reports are 0600 under umask 000 and 022, never overwrite destinations, and
  publish with directory synchronization.
- Runs without reports skip detailed report construction and memory hashing.
- Memory/configuration invariants are private and checked; controllers no longer
  receive Cpu; one orchestration path defines instruction time and interrupts.
- Errors/device records are typed internally and legacy reason strings remain
  stable at the JSON/CLI boundary.
- Reports contain versioned configuration, input/provenance, initial overlay
  identity, and explicit digest/truncation semantics.
- Compaction preserves source/base bytes, honors source locking, and produces an
  equivalent bootable journal at a fresh path.
- Existing tests and new integration/resource/lifecycle tests pass; CI covers
  the declared MSRV and stable Rust; provenance limitations are documented.

## Review and execution handoff

Review this written design before implementation planning. After approval, write
the implementation plan with ordered tasks, affected interfaces, regression gates,
and execution method. Changes must be checked against this acceptance checklist;
no finding is considered fixed merely because the original suite still passes.
