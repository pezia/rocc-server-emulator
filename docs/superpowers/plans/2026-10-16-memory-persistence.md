# Memory Persistence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Preserve RAM across orderly emulator exits so WMS can restart using its matching persistent disk overlay.

**Architecture:** Encode all physical memory in a checked binary sidecar derived from the locked disk overlay. Mark the sidecar active before execution, restore RAM before reloading the bootstrap, and atomically publish a committed snapshot after orderly execution. Keep CPU stepping and existing library constructors independent of persistence.

**Tech Stack:** Rust 2021, Rust 1.87 minimum, existing sha2/serde/libc dependencies, Python unittest, POSIX files and terminals.

**Spec:** `docs/superpowers/specs/2026-10-16-memory-persistence-design.md` (approved).

## Global Constraints

- The base image remains read-only, and the ROCCOVL1 disk format remains unchanged.
- Every CLI boot uses memory persistence by default.
- Do not add an independent memory filename override in this version.
- No CPU registers, bank selector, device queues, guest step counter, or screen contents are restored.
- Keep persistence out of `Machine::step`, CPU execution, DMA, and the terminal event loop.
- Create sidecars and publication temporaries privately with mode 0600.
- The largest current configuration has 585,728 physical words, with a 1,757,184-byte payload.
- Forced termination, process crashes, and host power failure do not guarantee a new RAM save.
- Preserve the existing cold-boot changes already present in this checkout; checkpoint them separately as the tested prerequisite before persistence task commits.
- Keep private images, RAM snapshots, and runtime reports outside the repository.
- Implementation starts after written-plan review and execution-method selection.

## Review Focus

1. A report filename equals an absent derived RAM filename: reject before creating or marking the sidecar (Task 2/3).
2. A killed process made no disk writes: the old RAM snapshot must still be rejected as interrupted (Task 2/3).
3. A `--no-memory` run changes disk state: the next persistent run must reject the old RAM (Task 3).
4. Snapshot rename succeeds but directory sync fails: report failure without deleting the complete new snapshot (Task 2).
5. Batch and interactive modes use different terminal settings: compatible RAM must restore, while CPU profile or address-width differences must fail (Task 1/3).

## File and interface map

- Modify `src/memory.rs`: bounded physical-memory payload import and export, preserving unknown words.
- Create `src/persistence.rs`: format, snapshot identities, protected-file checks, active marker, atomic replacement, unit tests.
- Modify `src/lib.rs`: private persistence module and public status re-export.
- Modify `src/overlay.rs`, `src/devices.rs`: crate-private identity access for the attached, locked overlay descriptor.
- Modify `src/machine.rs`: persistence attachment, bootstrap reload, snapshot finalization, read-only persistence status.
- Modify `src/main.rs`: default attachment, `--no-memory`, orderly finalization, summary output.
- Modify `src/report.rs`: persistence status in streamed reports.
- Create `tests/persistence.rs`: library lifecycle and retained-RAM restart tests.
- Modify `tests/cli.py`: process, signal, opt-out, alias, and recovery regressions.
- Modify `README.md`, `docs/machine-architecture.md`, `.gitignore`: user contract, format reference, and runtime-sidecar ignore rules.

## Execution preflight

- [x] Read this plan and the approved specification. Preserve the current checkout's uncommitted cold-boot work. Do not move it into another checkout.
- [x] Record `git status --short` and `git rev-parse HEAD`. Follow the worktree skill's existing-workspace and consent rules if isolation is needed; the user has not requested a worktree.
- [x] Run the existing baseline once: `cargo test --locked --offline`, `cargo build --release --locked --offline`, `python3 tests/cli.py`, and `python3 tests/regression.py --cpu`.
- [x] After those checks pass, commit only the already implemented cold-boot prerequisite (`README.md`, `docs/cpu-isa.md`, its source changes, `tests/cli.py`, and `tests/cold_boot.rs`) with `fix: recover WMS disk overlays through the cold boot panel sequence`. Preserve all bytes and leave unrelated files untouched. This separates the previously completed fix from persistence commits and keeps each task commit buildable.

The available toolchain is `/tmp/rocc-rustup/toolchains/1.87.0-x86_64-unknown-linux-gnu/bin` on `PATH`, with `CARGO_HOME=/tmp/rocc-cargo` and `RUSTUP_HOME=/tmp/rocc-rustup`. Use cached dependencies and avoid unrelated installations.

## Task 1: Checked RAM payload and versioned snapshot format

**Files:** Modify `src/memory.rs`, `src/lib.rs`; create `src/persistence.rs` with unit tests.

**Interfaces:**

- Produce crate-private `Memory::snapshot_bytes(&self) -> Vec<u8>` and `Memory::restore_snapshot_bytes(&mut self, payload: &[u8]) -> Result<(), String>`.
- Produce crate-private `SnapshotIdentity { profile: CpuProfile, address_bits: u8, image_bytes: u64, image_sha256: String, overlay_sha256: String }`. Hash strings must be exactly 64 hexadecimal digits before encoding.
- Produce crate-private `encode_snapshot(memory: &Memory, identity: &SnapshotIdentity) -> Result<Vec<u8>, String>` and `decode_snapshot(bytes: &[u8], memory: &Memory, identity: &SnapshotIdentity) -> Result<Vec<u8>, String>`.
- Produce public, serializable `MemoryStatus { enabled: bool, path: Option<PathBuf>, restored: bool, bypassed_for_cold_boot: bool, committed: bool, loaded_sha256: Option<String> }`, defaulting to disabled. Re-export it from `lib.rs`; keep codec and session internals private.

- [x] Add `physical_snapshot_preserves_unknown_zero_and_all_banks` inside `memory.rs`. Set known zero at physical word 0, `0x1234` at 10941, and `0xabcd` at the final physical word; leave word 1 unknown. Assert the extended payload has exactly `1_757_184` bytes, then round-trip and compare every `Option<u16>` from `iter_physical()`.
- [x] Add `invalid_memory_payload_is_atomic`: wrong length, invalid flag 2, and unknown flag 0 with nonzero value must each return an error and leave every existing cell unchanged.
- [x] Run `cargo test --locked --offline physical_snapshot` and the invalid-payload test; observe missing-method failures before implementation.
- [x] Implement both methods in `memory.rs`. Encode each cell as `[flag, high, low]`; validate the full payload into temporary storage before replacement. Do not change the live mapping during payload import.
- [x] Add codec tests with these exact assertions: total length equals `160 + payload.len()`; magic is `ROCCRAM1`; header version is 1; committed status is 0; changed image, overlay digest, profile, width, or geometry fails; a payload bit flip, truncation, trailing byte, nonzero reserved byte, or unsupported version fails. A terminal/capture-only configuration change must still decode.
- [x] Run `cargo test --locked --offline persistence` and observe failures before implementing the codec.
- [x] Implement the following fixed, big-endian header, with no new dependencies:

| Byte range | Value |
| --- | --- |
| 0..8 | ASCII `ROCCRAM1` |
| 8..10 | u16 format version 1 |
| 10 | status: 0 committed, 1 active |
| 11 | profile: 0 loader, 1 extended |
| 12 | address width: 15 or 16 |
| 13..16 | zero reserved bytes |
| 16..20 | u32 global word count |
| 20..24 | u32 words per local bank |
| 24..28 | u32 bank count |
| 28..32 | u32 physical word count |
| 32..40 | u64 base-image byte length |
| 40..72 | base-image SHA-256 bytes |
| 72..104 | effective overlay-state SHA-256 bytes |
| 104..128 | zero reserved bytes |
| 128..160 | SHA-256 of header bytes 0..128 followed by payload |
| 160..end | canonical physical-memory payload |

Ordinary decode recognizes an active marker before attempting committed checksum or payload validation and reports an interrupted session. Validate lengths and configured geometry before allocation. An active marker can be just a complete 160-byte header with status 1 and no payload. The codec never accepts that as restored RAM.

- [x] Run `cargo test --locked --offline`; require all old and new tests to pass.
- [x] Review the task's diff. Stage only this task's files and hunks, then commit with `feat: encode validated retained memory snapshots`.

## Task 2: Protected sidecar sessions and atomic snapshot replacement

**Files:** Modify `src/persistence.rs`, `src/overlay.rs`, `src/devices.rs`; add module-local file and failure-injection tests.

**Interfaces:**

- Consume Task 1 codec and `MemoryStatus`.
- Produce crate-private `FileIdentity { dev: u64, ino: u64 }` with an identity constructor from `Metadata`.
- Produce `ProtectedPaths { base: FileIdentity, overlay: FileIdentity, report: Option<PathBuf> }` for descriptor-derived protected identities and the already validated canonical report destination.
- Produce crate-private `OverlayFile::identity(&self) -> Result<FileIdentity, String>` and `Disk::overlay_identity(&self) -> Result<FileIdentity, String>`; the latter fails if no journal is attached.
- Produce crate-private `MemorySession::prepare(path: &Path, protected: &ProtectedPaths, memory: &Memory, identity: &SnapshotIdentity, cold_boot: bool) -> Result<(Self, Option<Vec<u8>>), String>`.
- Produce `MemorySession::begin(&mut self) -> Result<(), String>`, `MemorySession::commit(&mut self, memory: &Memory, identity: &SnapshotIdentity) -> Result<(), String>`, and `MemorySession::status(&self) -> &MemoryStatus`.

- [x] Add tests named `prepare_does_not_mutate_sidecar`, `begin_makes_previous_snapshot_unrestorable`, `cold_recovery_accepts_recognized_damaged_snapshot`, and `unrecognized_file_is_preserved`. Assert prepare leaves file bytes unchanged; begin durably sets byte 10 to 1; ordinary reopen refuses active state; cold prepare bypasses known damaged payload but never replaces an arbitrary file without matching supported magic/version.
- [x] Add `sidecar_paths_reject_aliases_and_special_files`: base/overlay hard links, symlinks, directories, FIFOs, multiply linked snapshots, and a report destination equal to the derived sidecar all fail before changing sidecar or protected-file bytes. A nonexistent report destination equal to the sidecar must also fail.
- [x] Run `cargo test --locked --offline persistence` and observe missing-session API or assertion failures.
- [x] Implement prepare using read/write, no-follow, nonblocking opens and descriptor metadata. Derive the path in the caller; require canonical parent plus filename. Check type, link count, protected identities, report destination, magic, and version before any mutation. For an absent sidecar, defer create-new until begin. Bound reads to `160 + configured_physical_words * 3 + 1`.
- [x] Implement begin: verify the pathname still identifies the prepared file, or create-new a private active header if absent. Set the active status through the open descriptor and sync it before returning; sync the parent directory after creation. Do not delete the file in Drop.
- [x] Add `commit_failure_preserves_active_or_complete_snapshot` with injected failures at write, file sync, rename, and directory sync. Before rename, assert the active marker remains; after rename, assert a complete decodable snapshot remains even if final directory sync reports an error. Existing protected files remain byte-identical, temporaries are cleaned, and `status.committed` stays false on every reported failure.
- [x] Implement commit: encode a complete status-0 snapshot, create a unique mode-0600 sibling temporary, write and sync it, verify the destination still matches the session-owned inode, rename over that recognized sidecar, then sync the parent directory. Set committed status only after all steps succeed. Keep report and overlay-compaction publication unchanged.
- [x] Add `sidecar_permissions_follow_private_umask_policy`: new sidecar and replacement must have no group/other access under umask 000 and 022. Add `destination_replaced_during_session_is_not_overwritten` by replacing the path with an unrelated file before commit; assert refusal and unchanged replacement bytes.
- [x] Run `cargo test --locked --offline` and `cargo clippy --all-targets --locked --offline -- -D warnings`; require success.
- [x] Review and commit only this task's changes with `feat: persist RAM through protected sidecar sessions`.

## Task 3: Automatic retained-memory boot lifecycle and reporting

**Files:** Modify `src/machine.rs`, `src/main.rs`, `src/report.rs`, `src/lib.rs`; create `tests/persistence.rs`; modify `tests/cli.py`.

**Interfaces:**

- Consume Tasks 1 and 2 interfaces and the existing sorted overlay digest.
- Produce public `Machine::attach_memory(&mut self, overlay_path: &Path, image: &File, report_path: Option<&Path>) -> Result<(), String>`; require zero executed steps and an attached overlay whose descriptor identity matches the path.
- Produce public `Machine::finish_memory(&mut self) -> Result<(), String>` and `Machine::memory_status(&self) -> &MemoryStatus`. Store an optional private `MemorySession` on `Machine`; unattached library machines return disabled status and finalization is a no-op.
- Produce a private `Machine::reload_bootstrap(&mut self)` helper shared with `attach_overlay`. It loads exactly words 0 through 127 from the effective boot block and changes no other RAM.
- Add report field `memory_persistence` containing `MemoryStatus`; do not change the report schema version for additive fields.

- [x] Add library test `retained_ram_restores_before_bootstrap`: save known zero, unknown words, bank 127, and word 10941; start a new machine and attach the same disk/memory pair. Assert retained physical values match, mapping is bank zero, CPU PC is 127 with reset registers/counters, and boot words come from the current effective block rather than saved RAM. Assert attachment after a guest step fails.
- [x] Add `library_without_attachment_stays_stateless`: existing constructors, step execution, `cpu-cases`, and finalization do not create sidecars or change reference output.
- [x] Run `cargo test --locked --offline --test persistence`; observe missing lifecycle API failures.
- [x] Implement attachment: canonicalize the locked overlay, derive its `.ram` path, construct protected identities and initial snapshot identity, prepare the session, import any validated RAM, reset mapping to bank zero, reload the effective bootstrap, and call begin. Preserve restored word 10941. Keep the session and overlay alive together on `Machine`.
- [x] Implement finalization: commit only when no guest fault exists and the stop reason is absent at a completed batch budget, or is `halt`, `step_limit`, `user_exit`, `input_eof`, or `signal`. Use the final effective overlay digest. Other stop/error paths retain the active marker. Propagate save errors rather than reporting success.
- [x] Add CLI tests `default_memory_changes_next_boot`, `custom_overlay_pairs_memory`, `no_memory_skips_load_and_save`, `stateless_disk_write_invalidates_saved_ram`, `cold_boot_replaces_recognized_bad_ram`, `report_alias_is_rejected_before_memory_begin`, and `report_failure_does_not_lose_ram_save`. Synthetic programs must assert a retained word affects the second process's accumulator or branch; check base-image bytes remain unchanged.
- [x] Run the new CLI tests against the old release binary; observe option or missing-sidecar failures before CLI integration.
- [x] Implement `--no-memory` as a flag and document it in help. After all existing image, report, and overlay path checks, attach memory unless disabled. After execution returns through terminal cleanup, finalize memory before report publication and signal-status return. Print one concise RAM status line in the boot summary.
- [x] Update `RunMetadata` or report plumbing only as needed to export the machine's persistence status; report enabled/path/restored/cold-bypass/committed/loaded identity. Saving a report must not be required for saving RAM.
- [x] Extend existing fault/retry and signal CLI tests for the new lifecycle. Runs whose purpose is stateless disk-journal recovery should explicitly use `--no-memory`; separately assert guest faults leave active RAM and normal restart refuses it. PTY signal tests must assert RAM commit plus termios restoration and exit status `128 + signal`.
- [x] Add `killed_session_without_disk_writes_requires_recovery`: first create a committed snapshot, then run a synthetic loop, wait until its sidecar marker is active, kill it, and assert the next ordinary boot refuses the snapshot without modifying it. Confirm `--cold-boot` can recover a suitable synthetic guest. Always clean up the child process.
- [x] Run `cargo test --locked --offline`, release build, `python3 tests/cli.py`, and `python3 tests/regression.py --cpu`; require all suites to pass with the frozen-reference checksum unchanged.
- [x] Review and commit only persistence changes with `feat: restore retained RAM during normal emulator boot`.

## Task 4: User contract and WMS acceptance verification

**Files:** Modify `README.md`, `docs/machine-architecture.md`, `.gitignore`; update this plan's completed checkboxes. Runtime evidence belongs under a fresh `/tmp/rocc-memory-acceptance-*` directory.

**Interfaces:** Consume the completed CLI and report contracts; no new runtime interfaces.

- [x] Update documentation: ordinary boots automatically pair `OVERLAY.ram`; an existing disk-only WMS overlay needs one cold boot; later launches omit `--cold-boot`; `--no-memory` disables both directions; interrupted or incompatible state requires explicit recovery. Explain reset versus full execution resume and the orderly-exit limit. Document the 160-byte header and canonical three-byte cell encoding in the architecture reference.
- [x] Add ignore patterns for RAM sidecars and their implementation-specific temporary filenames without ignoring public source or test fixtures.
- [x] Record SHA-256 of the original `fujitsu.img` and `fujitsu.img.overlay`, then copy only the overlay into the acceptance directory. Run cold startup against that copy with extended profile, experimental terminal, connection at 26,000,000, and a 27,000,000-instruction budget. Assert menu present, RAM committed, and cold recovery completed.
- [x] Start a separate process with the same copy and no `--cold-boot`. Assert `memory_persistence.restored == true`, `cold_boot_completed == false`, no fault, and WMS4 Main Menu. Repeat the normal boot once more.
- [x] Run the copied pair interactively through `./boot-rocc --interactive --overlay COPY`; wait for the menu, send Ctrl-], and assert exit 0, committed RAM, user-exit report, and restored terminal settings. Keep reports private.
- [x] Verify original image and overlay SHA-256 values are unchanged. Do not seed or alter the user's real RAM sidecar without separate authorization; provide the exact first-boot command instead.
- [x] Run final checks: `cargo fmt --check`, `cargo test --locked --offline`, `cargo clippy --all-targets --locked --offline -- -D warnings`, `cargo build --release --locked --offline`, `python3 tests/cli.py`, `python3 tests/regression.py --cpu`, and `git diff --check`. Require success before completion claims.
- [x] Use the requesting-code-review skill for one independent review of the whole persistence change, including startup ordering, alias checks, interrupted-state detection, cold recovery, and commit failure behavior. Address critical/important findings, rerun affected checks, and record the actual verification results.
- [x] Review and commit documentation only with `docs: explain retained RAM and WMS restart recovery`. Do not push or merge.

## Execution handoff

Recommended method: **Native**, implementing these four dependent tasks in this session followed by one independent review. The codec and session interfaces are shared across the boot lifecycle, so keeping the implementation context together avoids repeated handoffs.

The user approved this written plan and Native execution. Implementation and verification results are recorded in the execution ledger and Git history.


## Verification results

Implemented and verified in the existing submodule checkout. The final suite
contains 92 Rust tests and 42 CLI tests, plus 8,262 frozen and 610 independent CPU
reference cases. Formatting, Clippy with warnings denied, release compilation,
and diff checks passed. One independent review found a disk-overlay hard-link
namespace gap; its regression failed before the fix and passed after rejecting
multiply linked overlays during RAM attachment.

Private acceptance evidence is in `/tmp/rocc-memory-acceptance-u55w4x0g`.
A cold boot, two separate normal restarts, and an interactive normal restart
reached the WMS4 Main Menu. Normal restarts restored RAM without cold recovery;
Ctrl-] saved RAM and restored terminal settings. The original image and overlay
hashes remained unchanged. No real RAM sidecar was created for the user's original
overlay. No remote push or merge was performed.

The implementation retains the approved orderly-exit durability scope and the
demonstrated WMS reset model. Concurrent noncooperating state-file replacement and
library callers bypassing the machine lifecycle are outside the supported usage.
