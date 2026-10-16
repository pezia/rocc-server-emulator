# Emulator Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Implement every approved audit finding and architecture recommendation while preserving the inferred machine's guest behavior and existing disk data.

**Architecture:** Introduce validated immutable configuration and checked memory mapping, then narrow controller access and centralize instruction timing. Bound terminal resources, restore host terminal state on catchable signals, stream private reports, and export compact overlays without replacing their sources. Preserve compatibility through the frozen corpus and add independent synthetic integration coverage.

**Tech Stack:** Rust 2021, Rust 1.87 minimum, serde/serde_json, sha2, libc, Python 3 unittest, POSIX terminals/files, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-10-16-emulator-hardening-design.md` (approved by user).

## Global Constraints

- Preserve recovered guest instruction behavior, interrupt ordering, existing boot CLI defaults, and the ROCCOVL1 overlay format.
- No base image may be modified. Public Rust interfaces may change to enforce invariants; migrate all repository callers and document those changes.
- Default pending-event capacity is 4096.
- Bound display history independently to 1 MiB of bytes and 4096 packets in both batch and interactive modes.
- Reject zero limits and values exceeding documented ceilings: 65536 events, 64 MiB capture bytes, and 65536 packets.
- Return `terminal_event_limit` without partial effects; batch capture failures also precede transfer effects.
- Handle SIGINT, SIGTERM, and SIGHUP; return 128 + signal number after cleanup and requested report publication. Keyboard exits retain `user_exit`.
- Create temporary report files with mode 0600 from the first write, regardless of a permissive umask.
- Keep all 8262 existing frozen-reference cases and their checksum unchanged.
- Test Rust 1.87 and stable on Ubuntu using locked dependencies. Use synthetic temporary images only.
- Implementation starts only after written-plan review and selection of execution method. No implementation or subagent dispatch is authorized by creation of this plan alone.

## Review Focus

1. Two clock events due together with one remaining queue slot: no partially queued clock transaction (Task 3).
2. Guest fault after IRQ entry: interrupted PC, fetched instruction, and step accounting remain correct (Task 2).
3. Signal during setup or blocked output: restore installed handlers and terminal state without signal-handler I/O (Task 4).
4. Destination appears between validation and publication, or directory sync fails after linking: preserve existing data and report the actual publication outcome (Task 5).
5. Incomplete source journal tail during compaction: source bytes remain unchanged, while complete valid records determine output (Task 7).

## File and interface map

- `src/config.rs` (new): validated profiles, immutable configuration and resource limits.
- `src/fault.rs` (new), `src/lib.rs`: typed errors with stable external reason strings; module exports.
- `src/memory.rs`: private geometry/storage/mapping and checked access APIs.
- `src/cpu.rs`: instruction execution and CPU registers, with no terminal clock ownership.
- `src/devices.rs`: controllers accepting Memory rather than Cpu; typed disk states.
- `src/machine.rs`: shared step orchestration, boot state, summary, replay input tracking.
- `src/terminal.rs`: bounded typed events/packets, byte ring, incremental output digest.
- `src/signals.rs` (new), `src/interactive.rs`: scoped shutdown signals, terminal lifecycle, bounded delivered-key tracking.
- `src/artifact.rs` (new): reusable private, durable, no-overwrite file publication.
- `src/report.rs` (new), `build.rs` (new): borrowed streaming reports and build identity.
- `src/overlay.rs`: nonmutating locked snapshot reader and compact export.
- `src/main.rs`: validated CLI settings, summary/report split, compact-overlay dispatch.
- `tests/cli.py`, `tests/regression.py`: CLI regressions and bounded subprocess execution.
- `tests/integration.rs` (new): multi-instruction controller/IRQ/memory programs.
- `tests/reference_model.py`, `tests/generate_reference.py`, `tests/generated-reference.json`, `tests/REFERENCE.md` (new): independently specified reproducible corpus and provenance.
- `examples/performance.rs`, `tests/performance.py`, `docs/performance.md` (new): repeatable resource/throughput measurements.
- `.github/workflows/ci.yml` (new), `README.md`, `docs/machine-architecture.md`, `docs/cpu-isa.md`: automation and final contracts.

## Execution preflight

- [x] Read the approved spec and this plan; establish an isolated workspace using the worktree skill and retain the starting revision.
- [x] Capture Task 10's baseline measurements before editing source. Run the existing checks once in that workspace, recording toolchain, test counts, and any environmental failures.

## Task 1: Validated configuration and typed faults

**Files:** Create `src/config.rs`, `src/fault.rs`; modify `src/lib.rs`, `src/machine.rs`, `src/main.rs`, `src/cpu.rs`, `src/devices.rs`, `src/terminal.rs`, `Cargo.toml`; tests in new modules and `tests/cli.py`.

**Interfaces:**
- Produce `CpuProfile::{Loader, Extended}`, `TerminalProfile::{None, Experimental}`, `CaptureMode::{Stop, Tail}`.
- Produce `ResourceLimits::new(events: usize, bytes: usize, packets: usize) -> Result<Self, ConfigError>` and `Default`; private fields with getters.
- Produce `MachineConfig::new(profile: CpuProfile, address_bits: u8, terminal: TerminalProfile, connect_after: Option<u64>, limits: ResourceLimits, capture: CaptureMode) -> Result<Self, ConfigError>`; private fields with getters.
- Produce `Machine::with_config(raw: Vec<u8>, config: MachineConfig) -> Result<Self, String>`; existing `Machine::new` delegates to it.
- Produce `Fault { kind: FaultKind, context: FaultContext }`, `Fault::reason(&self) -> &'static str`, and `Fault::details(&self) -> serde_json::Value`. Context consists of typed memory, transfer, protocol, selector, descriptor, resource, or persistence variants rather than stored JSON. Enumerate all existing reasons; no arbitrary-string fallback. Add serde derive for typed export records, with a version compatible with Rust 1.87.

- [x] Write `config_rejects_invalid_profiles_and_limits`: assert invalid width/profile/connection combinations fail, defaults equal `(4096, 1048576, 4096)`, zero and ceiling+1 values fail, exact ceilings succeed. CLI test invalid limits leave overlay/report absent.
- [x] Run `cargo test --locked config` and the new CLI test after building release; confirm new API/flags fail before implementation.
- [x] Implement interfaces and `--event-limit`, `--capture-bytes`, `--packet-limit`; parse/validate before runtime artifacts. Inventory all existing Fault constructors and migrate to enum/context constructors while preserving details keys and reason strings. Preserve current fixture JSON.
- [x] Run `cargo test --locked`, release build, `python3 tests/cli.py`, and `python3 tests/regression.py --cpu`; require all existing checks plus new tests to pass.
- [x] Commit with `refactor: validate machine configuration and type emulator faults`.

## Task 2: Checked memory, narrow controllers, and one execution clock

**Files:** Modify `src/memory.rs`, `src/cpu.rs`, `src/devices.rs`, `src/machine.rs`, `src/main.rs`, `src/terminal.rs`; create `tests/integration.rs`.

**Interfaces:**
- Consume Task 1 configuration/fault types.
- Produce `Mapping::{SelectedBank(u8), GlobalOnly}` and `Memory::new(config: &MachineConfig) -> Self`; mapping changes use `set_mapping(&mut self, mapping: Mapping) -> Result<(), Fault>` so flat memory rejects nonexistent banks.
- Produce `read(&self, address: usize) -> Result<u16, Fault>`, `write(&mut self, address: usize, value: u16) -> Result<(), Fault>`, `peek(&self, address: usize) -> Result<Option<u16>, Fault>`, and immutable geometry getters `mask`, `logical_words`, `global_words`, `banks`.
- Produce `physical(&self, address: usize, mapping: Mapping) -> Result<usize, Fault>`, `validate_range(&self, start: usize, words: usize, mapping: Mapping) -> Result<(), Fault>`, checked `initialize_physical(&mut self, address: usize, value: u16) -> Result<(), Fault>`, and `iter_physical(&self) -> impl Iterator<Item = Option<u16>> + '_`. Physical access for validated block-copy staging remains crate private.
- Produce `Bus::io(&mut self, memory: &mut Memory, num: u8, op: u8, pulse: u8, value: u16) -> Result<IoResult, Fault>` and analogous Disk interface; no controller receives Cpu.
- Produce `Cpu::execute(&mut self, bus: &mut Bus) -> Result<StepInfo, Fault>` and `Machine::execute_step(cpu: &mut Cpu, bus: &mut Bus) -> Result<StepInfo, Fault>` for the shared tick/IRQ/execute path used by Machine and fixtures. Keep stepping orchestration out of Cpu.

- [x] Add `invalid_addresses_and_banks_return_faults`, `explicit_source_bank_forward_overlap_is_atomic`, and `irq_fault_preserves_actual_instruction`: assert address 65536 and bank 128 fail without panic; reject overflowed ranges including `usize::MAX`; preserve legitimate zero/end ranges; check global-only fault precedence and source-bank independence. Check IRQ save PC, failing handler word/PC, one tick/step, and no execution when clock faults.
- [x] Run `cargo test --locked` with these tests and observe missing API/assertion failures.
- [x] Implement interfaces; remove public mutable geometry/storage/global-only and duplicate configuration fields. Route DMA through checked selected-mapping ranges, with typed `DiskCompletion::{Idle, Control, Transfer { end: u16 }}`. Preserve partial register initialization and existing fault precedence. Move terminal tick/IRQ recognition to shared orchestration, migrate every fixture/test caller, and retain block-copy validation before writes.
- [x] Run Rust tests and both Python harnesses after a release build; frozen case count/checksum/results must remain identical.
- [x] Commit with `refactor: enforce memory invariants and centralize machine stepping`.

## Task 3: Bounded typed terminal events and capture

**Files:** Modify `src/terminal.rs`, `src/devices.rs`, `src/machine.rs`, `src/interactive.rs`; tests in `src/terminal.rs`, `tests/integration.rs`.

**Interfaces:**
- Consume validated ResourceLimits/CaptureMode and shared execute-step path.
- Produce `Terminal::with_config(config: &MachineConfig) -> Self`, fallible `tick(&mut self) -> Result<(), Fault>`, `connect(&mut self) -> Result<(), Fault>`, and existing fallible `send_key`.
- Private `TerminalEvent` enum supplies `(A,B,C)` values; private `DisplayPacket` and `RegisterOutput` types replace JSON records. Provide `pending_events`, `next_event`, `connected`, and read-only capture statistics for Bus/host callers. Retain screen snapshots at export boundaries.
- Typed `OutputState::{Unconfigured, Ready { dma: Option<u16> }}` (with separate non-output command state) determines immediate/DMA readiness. No unchecked optional-register unwrap.
- Produce retained-byte/packet truncation flags, retained/total counters, `full_stream_sha256` and `retained_sha256` getters for Task 6.

- [x] Add `event_limit_rejects_command_atomically`, `clock_events_reserve_capacity_together`, `timer_pending_flag_tracks_pop`, `batch_packets_and_bytes_have_independent_limits`, `live_history_evicts_and_keeps_full_screen`, and `failed_transfer_preserves_dma_and_ack_state`. With event capacity 1 and simultaneous connection/timer due, assert no event/connection/timer transition occurs before fault. Test exact limits, oversized single DMA output, and event removal via all clear operations.
- [x] Run `cargo test --locked terminal`; verify new saturation/packet tests fail against previous behavior.
- [x] Implement capacity preflight before all effects, including clock events as one transaction; update elapsed instruction time once even if capacity fails. Keep timer-pending state synchronized at queue removal. Use VecDeque bytes and typed packet records; stream evicted bytes into no extra storage. Track full-stream SHA incrementally. Batch stops before either cap; tail evicts with separate flags. Keep existing reason `terminal_capture_limit` for byte/packet capture exhaustion and `terminal_event_limit` for events.
- [x] Run terminal/integration tests, all Rust tests, and both Python harnesses; verify former 100000-event reproduction stops at the configured bound and screen state survives tail eviction.
- [x] Commit with `fix: bound terminal queues and diagnostic capture`.

## Task 4: Scoped signal shutdown and terminal cleanup

**Files:** Create `src/signals.rs`; modify `src/lib.rs`, `src/interactive.rs`, `src/main.rs`, `tests/cli.py`.

**Interfaces:**
- Produce crate-private `SignalSession::enter() -> io::Result<Self>`, `pending_signal(&self) -> Option<i32>`, and Drop restoring prior sigactions. Use a signal-safe atomic flag; reject concurrent host sessions in one process instead of clobbering global state.
- Produce typed `InteractiveOutcome { signal: Option<i32>, keys_delivered: u64, keys_pending: usize, notices: Vec<String> }`; main returns a numeric exit code after reports, while preserving existing CLI error formatting.

- [x] Add PTY tests `test_signals_restore_terminal_and_publish_report` for SIGINT/SIGTERM/SIGHUP: wait for raw-mode screen entry, send signal, drain output, assert exact original termios, report signal reason, and status `128 + signo`. Add handler-install/partial-setup restoration tests and a subprocess timeout for blocked terminal output.
- [x] Build release and run the new PTY tests; confirm the previous implementation fails restoration/status assertions.
- [x] Install handlers before raw mode; handler only stores the signal number. Use bounded nonblocking/poll-based host output writes that check shutdown, so a full output queue cannot indefinitely delay restoration. Check signals during setup/read/render and normal loop boundaries; cleanup restores termios even if rendering fails. Report terminal cleanup failures explicitly without masking the original failure. Make cleanup output best effort and bounded.
- [x] Run Rust signal guard tests and `python3 tests/cli.py`; verify keyboard Ctrl-C/Ctrl-] still exit successfully as user_exit and guest-fault restoration remains intact.
- [x] Commit with `fix: restore interactive terminals on termination signals`.

## Task 5: Shared private durable artifact publication

**Files:** Create `src/artifact.rs`; modify `src/lib.rs`, `src/main.rs`, `tests/cli.py`; unit tests in artifact module.

**Interfaces:**
- Produce `check_destination(path: &Path) -> io::Result<PathBuf>` and `publish_new<F>(dest: &Path, write: F) -> io::Result<()> where F: FnOnce(&mut File) -> io::Result<()>` in a library-exported artifact module usable by the separate main binary, reports, and overlay tooling. Use parent-relative unique create_new temp files at mode 0600 and no-overwrite hard-link publication on supported POSIX filesystems.
- Internal test-only file-operation injection covers write, file-sync, link, unlink, and directory-sync failures; no new production CLI testing flags.

- [x] Add `test_report_permissions_ignore_permissive_umask` for 000/022, existing-destination/symlink tests, and unit tests `raced_destination_is_preserved`, `writer_error_cleans_temporary`, `directory_sync_failure_is_reported`. Assert output permissions 0600 and no original bytes change. A failure after successful publication must leave the published complete file and return an error describing that outcome.
- [x] Run targeted tests after building release; confirm 0644 permissions under umask 022 before the fix.
- [x] Implement publication ordering: write, sync file, link without overwrite, remove temp, sync parent. On post-link cleanup failure still attempt directory sync; on any failure preserve published/existing destination and clean temp when possible. Validate destination before execution but rely on publication for race safety.
- [x] Run all artifact tests and CLI harness; verify output stays parseable/complete, never overwrites, and permissions are correct from creation.
- [x] Commit with `fix: publish runtime artifacts privately and durably`.

## Task 6: Lightweight summaries and versioned streaming reports

**Files:** Create `src/report.rs`, `build.rs`; modify `src/machine.rs`, `src/terminal.rs`, `src/devices.rs`, `src/interactive.rs`, `src/main.rs`, `Cargo.toml`, `tests/cli.py`.

**Interfaces:**
- Produce borrowed `RunSummary<'a> { reason: &'a str, pc: u16, steps: u64, disk_reads: u64, disk_writes: u64, overlay_blocks: usize, screen: &'a Screen }`, `Machine::summary(&self) -> RunSummary<'_>`.
- Produce `RunMetadata` containing paths, elapsed time, absolute budget, full batch schedule, bounded delivered-key history, signal and interactive outcome; `write_report<W: Write>(writer: W, machine: &Machine, metadata: &RunMetadata) -> io::Result<()>`.
- Schema version 2 retains legacy fields/reasons and adds configuration/build/input/digest-scope fields. Build revision comes from build.rs with an explicit unavailable fallback and dirty-tree indication when git is available; rerun on relevant git/worktree metadata changes.
- Replay history capacity is 65536 delivered interactive keys, evicting oldest entries with `replay_complete=false`; input records use `{step: u64, byte: u8}`. Batch schedules already present in CLI configuration are serialized directly, not duplicated into delivery history.
- Define overlay-state SHA-256 over ascending entries encoded as `offset:u64 big-endian || block:[u8;256]`, including a fixed format domain prefix `ROCC-OVERLAY-STATE-1`; empty state uses that prefix alone. Freeze initial digest after attach; report final separately.

- [x] Add `summary_does_not_visit_report_data` using test-only hash/export counters, `streamed_report_matches_legacy_fields`, `metadata_distinguishes_address_width_and_initial_overlay`, and `replay_and_digest_scopes_are_explicit`. Assert summary performs zero memory-hash/overlay-export operations; ensure tail eviction changes retained digest but not full-stream digest. Verify exact 65536-key history and truncation flag.
- [x] Run targeted tests; confirm old unconditional reporting/absent metadata fails new assertions.
- [x] Implement borrowed serde Serialize views/SerializeSeq for packets, memory-backed fields, and changed blocks. Hash memory with three-byte scratch per physical cell, keeping legacy encoding; encode each disk block with fixed 512-byte hex scratch. No whole-report Value or cloned disk map on normal CLI path. Serialize only when requested through Task 5 helper. Keep a convenience in-memory report API only for explicit library callers/tests if needed, delegating to the same serializer.
- [x] Run Rust tests and both harnesses; compare parsed legacy fields on synthetic runs and check report/no-report summaries agree. Verify build metadata fallback without a .git directory and normal worktree builds.
- [x] Commit with `refactor: stream versioned reports and avoid unused exports`.

## Task 7: Locked nonmutating overlay compaction

**Files:** Modify `src/overlay.rs`, `src/main.rs`, `src/lib.rs`; add CLI tests in `tests/cli.py`.

**Interfaces:**
- Produce `compact_overlay(image: &Path, source: &Path, destination: &Path) -> Result<CompactionSummary, String>` and `CompactionSummary { original_bytes: u64, compacted_bytes: u64, blocks: usize }` through a narrow library export.
- Factor source replay into `ReplayPolicy::{RecoverTail, PreserveSource}`: boot retains existing partial-tail recovery; compaction rejects invalid complete records but ignores incomplete trailing bytes without truncating the source. Compaction never creates a missing source.
- Consume Task 5 publication helper; source lock guard remains alive until destination publication/directory sync completes.

- [x] Add tests `test_compaction_preserves_source_and_boot_state`, `test_compaction_honors_lock_and_rejects_aliases`, `test_compaction_rejects_corrupt_source_without_output`, `test_compaction_preserves_incomplete_tail`, plus injected destination-write/sync failure tests. Repeated writes to one block compact to `48 + 296` bytes; source/base hashes remain unchanged; reboots produce equivalent state.
- [x] Run new CLI tests after a release build and observe the absent command fail.
- [x] Implement exact CLI `compact-overlay IMAGE --overlay SOURCE --output NEWPATH`; validate image, source identity/regular-file/no-symlink policy, header/checksums/offsets and new destination. Lock source nonblocking, then write deterministic latest-block records to private output. Never replace original journal; print byte/block statistics.
- [x] Run full CLI suite plus Rust overlay/artifact tests, including contention and a source with an incomplete final append; require no source-byte mutation in every compaction outcome.
- [x] Commit with `feat: export compacted disk overlay journals safely`.

## Task 8: Independent reference generation and integrated machine tests

**Files:** Create `tests/reference_model.py`, `tests/generate_reference.py`, `tests/generated-reference.json`, `tests/REFERENCE.md`; expand `tests/integration.rs`, `tests/regression.py`, `tests/cli.py`.

**Interfaces:**
- Python `generate(seed: int = 1380926275) -> dict` returns deterministic corpus metadata/cases. Reference functions evaluate arithmetic through independently expressed bit/carry operations and addressing through explicit signed displacements; never import/call the emulator.
- `python3 tests/generate_reference.py --check` compares checked-in generated corpus bytes; default writes the new corpus only. JSON uses sorted keys/fixed indentation and no time-dependent metadata.
- Extend `tests/regression.py --cpu` to validate both the unchanged frozen corpus and the new corpus and report their counts separately; all subprocess.run/communicate/wait operations in harnesses receive bounded timeouts (CPU regression 120 seconds, short CLI operations 15 seconds).

- [x] Specify new corpus hand-check cases: ADD `ffff+1` with carry variants, SUB equal operands, 17-bit rotates, NL preserving registers/carry, signed relative wrap, autoindex original continuation bit. Add integration programs for ION delay/masked disk-vs-terminal priority, bank-switch/DMA/readback, terminal event tuple consumption, and a combined timer/IRQ/bank/DMA sequence. Pin expected transitions from documented semantics, including zero-count block move and exact-end/out-of-bounds/unknown-source DMA.
- [x] Run these named tests/corpus comparison before generator implementation to establish missing coverage; do not regenerate or edit the old gzip corpus/checksum.
- [x] Implement independent model/generator and program builders; verify model basics with hand-calculated assertions before generating broader deterministic cases. Document that legacy oracle provenance remains unknown beyond its existing seed/description. Add state-transition/atomicity assertions for new typed terminal/disk protocols.
- [x] Run generator `--check`, Rust integration tests, release build, both harnesses. Check SHA256 of decompressed frozen corpus remains `f0e281511050716f1ea3f8b53f321b3bd896f7a209b264427d5a7df68651ae07` and count remains 8262.
- [x] Commit with `test: add reproducible reference and machine integration coverage`.

## Task 9: MSRV and stable continuous integration

**Files:** Create `.github/workflows/ci.yml`; modify `Cargo.toml`/`Cargo.lock` only if minimum-version compatibility requires it.

**Interfaces:** Workflow push/pull_request jobs use read-only contents permissions; Ubuntu matrix Rust `1.87.0` and `stable`, Python 3, locked dependencies; stable-only lint job.

- [x] Check available toolchains with `rustup toolchain list`; attempt `cargo +1.87.0 test --locked` and release build. If installation/network access is required, use the normal permission mechanism and report genuine environment blockers without claiming verification.
- [x] Add matrix commands `cargo test --locked`, `cargo build --release --locked`, `python3 tests/cli.py`, `python3 tests/regression.py --cpu`, and generator `--check`; stable lint commands `cargo fmt --check` and `cargo clippy --all-targets --locked -- -D warnings`. Set job timeouts; no credentials/private image dependencies.
- [x] Resolve demonstrated MSRV dependency incompatibilities by choosing compatible versions and updating the lockfile, keeping rust-version 1.87. Do not alter dependencies speculatively.
- [x] Run the complete matrix locally where toolchains are available; inspect workflow commands against tested invocations. State clearly if remote Actions execution has not occurred.
- [x] Commit with `ci: verify emulator on minimum and stable Rust`.

## Task 10: Performance evidence, documentation, and whole-change verification

**Files:** Create `examples/performance.rs`, `tests/performance.py`, `docs/performance.md`; update `README.md`, `docs/machine-architecture.md`, `docs/cpu-isa.md`, CLI help and source-layout documentation.

**Interfaces:** `cargo run --release --example performance -- CASE ITERATIONS` supports `instructions`, `terminal-tail`, and `report`; Python driver runs those and overlay replay/compaction scenarios using temporary synthetic images and records environment, throughput, elapsed time and peak RSS where available. No correctness test has machine-specific timing thresholds.

- [x] Before modifying source during execution, capture baseline release throughput/output/report/replay measurements from the pre-implementation commit in an isolated temporary checkout; preserve command/environment/result data for final comparison. The original unbounded-event reproduction must remain a bounded-size probe.
- [x] Implement probes using stable public APIs after Tasks 1–8. Measure steady-state one-byte output after filling history, large changed-block report export, and equivalent overlay replay before/after compaction; distinguish I/O sync cost from instruction throughput.
- [x] Update docs for exact flags/defaults/ceilings, overflow policies, signal statuses/limits, report schema/digests/replay completeness, compaction usage and source preservation, checked Rust APIs, one execution clock, generation provenance, and CI commands. Replace old claims about unbounded packets or only normal-exit cleanup.
- [x] Run final `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked`, release build, both Python harnesses, generator `--check`, MSRV checks, performance probes, and `git diff --check`. Review each spec acceptance item against concrete test or measurement output. Obtain an independent whole-change review when the chosen execution method authorizes it; fix demonstrated defects and rerun affected checks.
- [x] Commit with `docs: document hardened emulator contracts and performance`; report exact passing counts, remaining environment limitations, benchmark results, and all public compatibility changes. Do not merge or push unless separately authorized.

## Execution handoff and self-review

Coverage: spec section 1 maps to Tasks 1–2; section 2 to Task 3; section 3 to Task 4;
section 4 to Tasks 5–6; section 5 to Task 7; section 6 to Tasks 8–10. Each review
focus item has a named regression in its owning task. Task 10 baseline capture is
an execution preflight performed before Task 1 edits. Shared names/signatures and
reason strings are defined above; serialization additions preserve legacy fields.

Recommended execution method: **Native**, because configuration, memory, timing,
terminal state, and reporting share interfaces across most tasks. One implementer
can preserve those contracts with less handoff overhead. Subagent-driven execution
is available if the user prefers per-task independent review. Wait for the user to
review this written plan and choose the execution method before implementation.


## Execution result

Completed natively on branch `emulator-hardening`. A fresh independent reviewer
identified three important issues and two smaller omissions. All five were fixed:
blocked post-session output, missing reports on host-key queue faults, arbitrary
JSON fault construction/null diagnostics, missing total packet counts, and an
interactive capture-policy mismatch. Reproductions were observed failing before
fixes; the complete suites were then rerun. Final evidence is recorded below.

- 70 Rust tests pass on stable and Rust 1.87.
- 28 CLI/PTY/persistence tests pass; 8262 frozen and 610 independent CPU cases pass.
- Five independent-model hand checks and generator reproducibility pass.
- Formatting and Clippy with warnings denied pass; performance probes run.
- GitHub Actions is configured but has not been run remotely.

Decisions and limits: sandbox-inaccessible managed worktree prompted the branch
fallback in the original checkout; Cpu::step remains a compatibility forwarding
method; remaining baseline measurements used an archived pre-change revision.
The review's two smaller omissions were included in the user's request for all
findings. Hardware fidelity, actual power-cut behavior, and remote CI execution
remain outside the evidence established by local synthetic checks.
