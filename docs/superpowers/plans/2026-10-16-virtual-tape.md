# Virtual Tape Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run the original WMS tape backup and restore through one file-backed Tape Unit 1 and verify restored guest contents on a disposable destination.

**Architecture:** Separate a bounded SIMH tape medium from the evidence-backed ROCC controller at octal device 20. Recover the hardware contract first; implement media, persistence, controller, and boot attachment in dependent tasks. Preserve absent-device behavior and validate media before activating retained RAM.

**Tech Stack:** Rust 2021, Rust 1.87 minimum, existing serde/serde_json/sha2/libc dependencies, Python unittest and the parent workspace's independent tape inspector. No new dependency is planned.

**Spec:** `docs/superpowers/specs/2026-10-16-virtual-tape-design.md` (approved).

## Global Constraints

- The first version supports one mounted virtual reel on WMS Tape Unit 1.
- Use disposable copies for all private-image investigation and acceptance tests.
- Keep original images, RAM snapshots, credentials, backup contents, and private traces out of Git.
- Preserve the immutable base disk, disk-overlay format, retained-RAM contract, and all existing guest behavior when no tape is attached.
- Cap tape files at 512 MiB, individual records at `0xFFFFFF` bytes, and parsed objects at 100,000.
- Add mutually exclusive `--tape PATH` and `--tape-create NEWPATH` options to boot. Both require the extended profile.
- `--tape` mounts an existing image read-only; `--tape-create` creates a new writable image without overwriting any existing path.
- Validate and open tape media before marking RAM active or executing any guest instructions.
- Flush complete framed records and tape marks before acknowledging guest-visible write completion.
- A mounted medium starts at beginning of tape.
- Unknown commands, pulse combinations, invalid unit encodings, or transfer modes remain explicit typed faults.
- Complete an independent review before reporting tape backup and restore as supported.
- Plan approval precedes execution. Preserve the user's previously selected Native execution method unless they change it.

## Review Focus

1. A report names an absent tape-create destination: reject before creating either artifact or activating RAM (Task 5).
2. A tape suffix cannot be fully committed after an earlier-record overwrite: report failure and never acknowledge a successful command (Tasks 3–4).
3. Guest requests data that ends outside mapped memory or includes an unknown source word: validate the whole transfer before modifying RAM or media (Task 4).
4. RAM was saved while a reel was positioned in the middle: the next launch mounts BOT with reset controller state (Tasks 4–5).
5. A structurally valid backup omitted the data under test: restore comparison must detect the omission rather than pass because the destination inherited the source (Task 6).

## File and interface map

- Create `docs/tape-controller.md`: command/status provenance and supported transfer contract; no private payloads or authentication instructions.
- Create `src/tape.rs`: medium/controller module boundary, shared errors and status re-exports.
- Create `src/tape/medium.rs`: bounded record index, framing, cursor, read and spacing semantics.
- Create `src/tape/session.rs`: file ownership, protected paths, locks, durable modification and media identity.
- Create `src/tape/controller.rs`: proven register/pulse state machine, checked transfers, reset and interrupts.
- Modify `src/lib.rs`, `src/fault.rs`, `src/devices.rs`: module export, typed faults, optional attachment, reset and interrupt routing.
- Modify `src/machine.rs`, `src/main.rs`, `src/report.rs`: pre-execution attachment, CLI validation, and borrowed diagnostics.
- Create `tests/tape.rs`, `tests/tape_controller.rs`, `tests/fixtures/tape-controller.json`: synthetic integration tests and sanitized protocol fixtures.
- Modify `tests/cli.py`, `README.md`, `docs/machine-architecture.md`, `.gitignore`: CLI regressions, supported user routes and artifact exclusions.

Keep medium and session types crate-private; expose controller attachment options and report status from `tape`. Keep CPU execution unchanged except existing bus delegation. Do not split unrelated disk/terminal code.

## Preflight

- [ ] Read this plan and its approved spec. Record `git status --short` and `git rev-parse HEAD`; preserve unrelated work. Apply the worktree skill's existing-workspace rules before execution.
- [ ] Set toolchain PATH to `/tmp/rocc-rustup/toolchains/1.87.0-x86_64-unknown-linux-gnu/bin`, `CARGO_HOME=/tmp/rocc-cargo`, and `RUSTUP_HOME=/tmp/rocc-rustup`.
- [ ] Establish a baseline with `cargo test --locked --offline`, `cargo build --release --locked --offline`, `python3 tests/cli.py`, and `python3 tests/regression.py --cpu`. Record results; diagnose failures before changing behavior.

## Task 1: Recover and freeze the ROCC tape contract

**Files:** Create `docs/tape-controller.md`, `tests/fixtures/tape-controller.json`. Investigation helpers remain outside Git under `/tmp/rocc-tape-investigation`.

**Interfaces:** Produce a provenance table and JSON fixture array. Each case has `name`, `evidence` (guest word addresses and instruction words), `initial_memory` (synthetic address/value pairs), `events` (operation, pulse, value, expected input/skip, and expected busy/done/IRQ), and `final_memory`. Fixtures contain synthetic data only. Task 4 consumes these exact keys and the table, rather than inferring values from a generic Nova controller.

- [ ] Refresh disposable overlay/RAM copies from the originals and record SHA-256 for all original inputs. Never continue from a RAM copy left active by an earlier probe.
- [ ] Reproduce DSAVE's first unsupported `DOAC AC3, 0o20` at `0xBD37`, instruction `0x7A90`, value `0x1000`, without skipping instructions. Record startup `DOAS 0x0810`, SKPS and DIAC separately.
- [ ] Disassemble the reachable tape driver, including its self-modifying I/O templates. Map DOA/B/C, DIA/B/C, pulse, busy/done tests, units, command fields, status bits, DMA or programmed I/O, word/byte counts, residual counts, and mapping behavior. Link every table row to concrete driver instructions.
- [ ] Trace the original restore selection on disposable state. Establish which save contents it consumes and which destination initialization the guest requires. Identify rewind, record transfer, tape marks, spacing, error handling and completion used by both routes.
- [ ] Investigate IRQ request, priority, mask, acknowledgement and CPU reset using the guest interrupt driver. If the actual tape route is polled, prove that and document why no tape IRQ is asserted for that route; do not assign an arbitrary mask bit.
- [ ] Run throwaway hypotheses only on copied state. Label supplied status values and skipped outputs as hypotheses until matched by the driver's tests and subsequent transfers. Stop at unknown data inputs instead of manufacturing successful reads.
- [ ] Write the provenance table and synthetic fixtures for every required operation, including not-ready and write-protected handling. Record explicit unsupported operations and confidence limits.
- [ ] Verify original hashes and inspect the staged diff for credentials, copied guest code, image bytes and private paths. Commit only sanitized evidence with `docs: recover WMS tape controller protocol`.

**Dependency gate:** Tasks 2–3 can implement the approved standard medium independently, but Task 4 must not invent any missing hardware values. If the investigation cannot establish a required transfer/completion contract, report the exact boundary and keep the controller unready. A contradiction with the approved architecture requires a revised design, not a guessed implementation. Documentation/medium work alone does not satisfy tape support.

## Task 2: Bounded SIMH medium and cursor

**Files:** Create `src/tape.rs`, `src/tape/medium.rs`; modify `src/lib.rs`; add module unit tests.

**Interfaces:** Define crate-private `TapeObject::{Data { offset: u64, length: u32 }, Mark}`, `TapeRead::{Data(Vec<u8>), Mark, End}`, `TapeStep::{Record { bytes: u32 }, Mark, End}`, and `Direction::{Forward, Reverse}`. Produce `TapeMedium::scan(file: &mut File) -> Result<Self, String>`, `read(&mut self, file: &mut File) -> Result<TapeRead, String>`, `space(&mut self, direction: Direction) -> Result<TapeStep, String>`, `rewind(&mut self)`, and `position(&self) -> usize`. Index metadata, not all record payloads. Spacing reports the crossed record length without reading its payload; crossing BOT returns `End` without moving the cursor.

- [ ] Add `standard_frames_and_cursor`: bytes `03 00 00 00 61 62 63 00 03 00 00 00 00 00 00 00` parse as data `abc` then a mark; assert position 0 initially, 1 after data, 2 after mark, stable at End, reverse crosses mark to position 1, rewind returns 0. Add even-length data and multiple files.
- [ ] Add `rejects_unsupported_or_malformed_objects`: mismatched trailer, truncated prefix/data/pad/trailer, error flag, extended object class and erase gap fail; arbitrary odd pad is accepted. Recognize `0xFFFFFFFF` and physical EOF as EOM, reject bytes following explicit EOM in this supported subset. Empty physical media is valid BOT/EOM.
- [ ] Add `resource_ceilings_precede_allocation`: reject file size `512*1024*1024+1`, record length above `0xFFFFFF`, and the 100,001st object. Exercise exact accepted boundaries using sparse files or bounded readers rather than allocating maximum payloads repeatedly.
- [ ] Run `cargo test --locked --offline tape::medium`; observe missing-module/API failures.
- [ ] Implement the declared types and methods using checked offset arithmetic and standard little-endian framing. Preserve tape marks as objects; zero length is never an empty data record.
- [ ] Run the medium tests and independently inspect an equivalent small fixture with the existing Python `TapeImage`. Commit with `feat: parse bounded SIMH tape media`.

## Task 3: Owned tape files and durable record writes

**Files:** Create `src/tape/session.rs`; extend `src/tape.rs`, `src/tape/medium.rs`; unit tests in the session module.

**Interfaces:** Produce public `TapeMount::{ReadOnly(PathBuf), CreateNew(PathBuf)}`. Crate-private `TapeProtectedPaths { image: PathBuf, overlay: PathBuf, ram: PathBuf, report: Option<PathBuf> }`, `TapeSession::open(mount: &TapeMount, protected: &TapeProtectedPaths) -> Result<Self, String>`, `read(&mut self) -> Result<TapeRead, String>`, `space(&mut self, direction: Direction) -> Result<TapeStep, String>`, `rewind(&mut self)`, `write_record(&mut self, bytes: &[u8]) -> Result<(), String>`, `write_mark(&mut self) -> Result<(), String>`. The session owns the locked descriptor, medium index, and write-failure latch. Empty `write_record` fails; use `write_mark` for zero framing.

- [ ] Add tests for read-only immutability; no-clobber creation with mode `0600`; symlink/nonregular rejection; canonical and inode aliases against all protected paths; lock contention; and alias checks for nonexistent destinations. Assert no file creation on validation errors.
- [ ] Add `durable_frames_before_ack`: write odd data `abc` and a mark, reopen and parse exact bytes from Task 2, assert zero pad and intact trailer. Inject record-write, sync and directory-sync failures; no successful return or subsequent successful operation after a partial write.
- [ ] Add append scaling instrumentation: writing N records performs O(N) total frame bytes, not repeated whole-file copying. Enforce the exact file/object/record ceilings before modification.
- [ ] Add `overwrite_suffix_failure_is_not_success` for the overwrite command only if Task 1 proves it is needed. Rewind then replace at a record boundary and truncate the old suffix; any write/truncation/sync failure latches failed media and prohibits completion. Read-only source remains unchanged. Do not add an unused overwrite feature.
- [ ] Run `cargo test --locked --offline tape::session`; observe failures, then implement the declared methods. Use descriptor identity checks, `O_NOFOLLOW`, `create_new`, nonblocking exclusive advisory locking for writable reels and file/directory sync for new-file durability. Keep the writable inode locked throughout the session.
- [ ] Run medium/session tests. Independently parse a Rust-produced temporary image with the Python inspector. Commit with `feat: persist locked virtual tape records`.

## Task 4: Evidence-backed tape controller and bus integration

**Files:** Create `src/tape/controller.rs` with unit tests; modify `src/tape.rs`, `src/devices.rs`, `src/fault.rs`; consume Task 1 fixtures.

**Interfaces:** Produce crate-private `TapeController::new(session: TapeSession) -> Self`, `io(&mut self, memory: &mut Memory, op: u8, pulse: u8, value: u16) -> Result<IoResult, Fault>`, `reset(&mut self)`, `tick(&mut self) -> Result<(), Fault>`, and `interrupt_pending(&self, mask: u16) -> bool`. Add `Bus::tape: Option<TapeController>` crate-private. Expose public serializable `TapeStatus` with mode, unit, initial/final SHA-256, records, marks, position, reads, writes, error and bounded recent events (maximum 256, no payloads). `TapeController::status(&self) -> &TapeStatus` returns cached diagnostics; hashing happens at mount/finalization, never per CPU step.

- [ ] Add fixture-driven tests before each command implementation: replay Task 1 events and assert inputs, skips, busy/done, IRQ and memory effects. Include boot discovery, reset, unit 1 ready, other valid units unavailable, rewind, record read/write, mark, spacing and short-record outcomes actually required by Task 1.
- [ ] Add `transfer_validation_is_atomic`: out-of-range/mapping-crossing destination leaves all RAM unchanged; unknown source words leave media unchanged; odd byte counts and short records produce the documented residual/status. Use `Memory::validate_range` plus a complete source staging read before committing effects.
- [ ] Add `unsupported_tape_operations_have_context`: unknown operation/pulse/command faults include device `0o20`, operation, pulse and output value. Add tape-specific fault reasons in `fault.rs`, preserving existing reasons for the absent port.
- [ ] Run `cargo test --locked --offline tape::controller`; observe failures, then implement each proven register/pulse case independently. Use Task 3 session operations and Task 1 byte order/count semantics. Keep unknown cases as faults rather than generic ready/done returns.
- [ ] Route extended device `0o20` to the controller only when mounted; retain the existing absent branch otherwise. Reset controller state in both CPU-reset paths. Wire tick only if the recovered completion model requires it; route IRQ arbitration at the established priority/mask, with existing disk and terminal behavior preserved.
- [ ] Add tests for simultaneous device requests, tape interrupt masking/acknowledgement, reset without erasing data, and unmounted behavior under loader/extended profiles. For a proved polling-only controller, assert it cannot introduce an IRQ.
- [ ] Run controller tests, `cargo test --locked --offline`, and `python3 tests/regression.py --cpu`. Commit with `feat: emulate recovered ROCC tape controller`.

## Task 5: Boot attachment, CLI, and diagnostics

**Files:** Modify `src/machine.rs`, `src/main.rs`, `src/report.rs`, `src/tape.rs`; create `tests/tape.rs`, `tests/tape_controller.rs`; extend `tests/cli.py`.

**Interfaces:** Produce `Machine::attach_tape(&mut self, mount: TapeMount, image_path: &Path, overlay_path: &Path, report_path: Option<&Path>) -> Result<(), String>`, `Machine::finish_tape(&mut self) -> Result<(), String>`, and `Machine::tape_status(&self) -> Option<&TapeStatus>`. Attachment rejects loader profiles, repeated attachment, nonzero steps and an already-active RAM session. Derive the protected RAM path using the same canonical overlay + `.ram` rule as memory attachment, including `--no-memory` runs.

- [ ] Add CLI cases for missing/duplicate/conflicting tape options, loader profile, absent read-only input, existing create destination, malformed image and every protected path alias. Include report = absent tape-create path; assert neither artifact exists and original RAM bytes are unchanged after failure.
- [ ] Add public-API fixture replay tests in `tests/tape_controller.rs` using mounted `Machine` and bus I/O. Add library lifecycle tests: tape attaches after overlay and before memory; memory restore does not alter mounted BOT; tape attachment after RAM activation/stepping fails; no tape retains existing report/CPU behavior.
- [ ] Run `cargo test --locked --offline --test tape` and `python3 tests/cli.py`; observe failures before implementing attachment and option parsing. Validate all path conflicts before creating a new tape. Attach tape before `attach_memory` in `main.rs`.
- [ ] Finalize tape diagnostics after guest execution and before publishing reports. Attempt RAM finalization even if tape finalization fails, then return the tape error without claiming completed backup. Record report `tape` as null or the status object, excluding payloads; bound diagnostics independently of tape length.
- [ ] Add tests for report counts/identities/position, bounded 256 events, finalization failure, interactive orderly exit and signal path. Distinguish durable recorded media from guest-reported backup completion in text output.
- [ ] Run targeted tests and the full Rust/CLI suite. Commit with `feat: mount virtual tapes through emulator boot`.

## Task 6: Guest backup/restore acceptance, documentation, and review

**Files:** Modify `README.md`, `docs/machine-architecture.md`, `.gitignore`, and `docs/tape-controller.md`; acceptance scripts/artifacts stay outside Git. Change source only for acceptance failures, with a failing synthetic regression first.

**Interfaces:** Consume Task 5 CLI and diagnostics. Produce a sanitized acceptance account naming exact guest save/restore routes, promised contents, verified comparison scope, completion evidence, limitations and verification results. Keep the detailed inventory and authentication local.

- [ ] Use copied private disk/RAM state to run original DSAVE with `--tape-create`. Capture guest completion, tape counts and committed SHA-256; inspect every generated record with the independent Python parser. Structural parsing alone is not acceptance.
- [ ] Prepare a separate disposable destination with at least one backed-up library/file absent or changed. Record that difference before restore. Mount the new tape read-only and execute the original restore route; compare all contents/metadata promised by the selected save operation. Require the deliberately missing/changed data to be restored, preventing an untouched-clone false positive.
- [ ] If the operation saves system-recovery data, boot the restored destination and verify its guest state. If it saves only selected files, document that scope and do not claim full-disk recovery. Any unsupported required command returns to Task 1 and gains evidence plus regression tests before implementation.
- [ ] Recheck hashes of original disk, overlay, RAM and read-only tape. Test remount starts BOT and a new launch cannot resume a controller operation from retained RAM.
- [ ] Document exact mount flags, read-only restore, new-file backup, single unit, supported SIMH subset/limits, guest routes, partial-backup/power-failure behavior and portability. Add ignore rules for generated tape artifacts without ignoring synthetic fixtures. Preserve the existing README warning at the beginning.
- [ ] Run `cargo fmt -- --check`, `cargo clippy --locked --offline --all-targets -- -D warnings`, `cargo test --locked --offline`, `cargo build --release --locked --offline`, `python3 tests/cli.py`, and `python3 tests/regression.py --cpu`. Record actual results rather than expected counts.
- [ ] Request one independent whole-branch review under the preserved Native method using the requesting-code-review skill. Review the approved spec, provenance, media failures, path aliases, transfers, absent behavior and acceptance evidence. Fix findings with focused regressions, rerun affected checks and obtain confirmation on material fixes.
- [ ] Commit documentation with `docs: describe verified WMS tape backup and restore`. Present verified behavior, test evidence and remaining limitations. Do not push unless the user requests it for this feature.
