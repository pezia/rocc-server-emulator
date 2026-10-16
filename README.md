# ROCC emulator

An experimental Rust emulator for inferred ROCC hardware: CPU, banked memory,
disk controller, interrupts, and a 24×80 terminal. Hardware behavior is still
being reconstructed; unsupported instructions and controller operations stop
execution with diagnostics.

No operating system, disk images, application data, or credentials are included.
Supply your own compatible image explicitly.

## GitHub Pages

The [public introduction](docs/index.html) explains the ROCC machine, links to
historical archive and museum records, and reserves galleries for owner-supplied
machine photos and real-system screenshots. See the
[GitHub Pages guide](docs/github-pages.md) to enable publishing and add images.

## Build and run

Requires Rust 1.87 or later and a C linker. Interactive mode requires a POSIX
terminal at least 80 columns wide and 24 rows high.

```sh
cargo build --release --locked
./target/release/rocc-emulator boot /path/to/disk.img --interactive
```

The input image is read-only. Guest writes are saved separately in
`/path/to/disk.img.overlay` and loaded automatically on subsequent runs. Override
that filename with `--overlay /path/to/session.overlay`. The overlay is required
and must be writable; the original image is never modified. Guest time is
instruction-based, not cycle-accurate.

Each disk write is flushed to the overlay before the guest receives completion.
The overlay is checked against the original image's size and SHA-256, and only
one emulator may use it at a time. Use a new overlay filename to start from the
original disk state, or keep separate overlays for independent sessions. See
[persistent disk overlays](docs/machine-architecture.md#persistent-disk-overlays)
for the file format and recovery behavior.

For batch execution:

```sh
./target/release/rocc-emulator boot /path/to/disk.img \
  --profile extended --terminal experimental \
  --connect-after 26000000 --steps 26200000 \
  --report report.json
```

Connection timing and instruction budgets are configurable. Run `--help` for
all options. Reports must use a new filename and can contain guest memory,
display output, image hashes, input paths, and modified disk blocks; treat them
as private runtime artifacts.

Terminal resources have independent limits in batch and interactive modes:

| Option | Default | Allowed range |
| --- | --- | --- |
| `--event-limit N` | 4096 pending events | 1–65536 |
| `--capture-bytes N` | 1048576 retained bytes | 1–67108864 |
| `--packet-limit N` | 4096 retained packets | 1–65536 |

A full event queue stops with `terminal_event_limit` before command effects.
Batch capture stops with `terminal_capture_limit` before exceeding either history
limit. Interactive capture evicts old history while preserving the current screen.

To compact repeated writes into a new overlay:

```sh
./target/release/rocc-emulator compact-overlay /path/to/disk.img \
  --overlay /path/to/session.overlay --output /path/to/compact.overlay
```

Compaction locks and validates the source, preserves it unchanged (including an
incomplete trailing record), and writes only the latest version of each block.
Select the new file with `--overlay` on the next boot. Existing destinations are
never overwritten.

Reports are created privately (`0600`, restricted further by the umask), streamed
to a temporary file, synced, and published without overwriting another file.
Report schema 2 records effective configuration, build revision, scheduled input,
initial/final overlay identities, and separate full-stream/retained-output hashes and total/retained packet counts.
Interactive delivered-key history retains 65536 entries; `replay_complete` records
whether earlier entries were discarded. Reports are diagnostics, not checkpoints.
Runs without `--report` skip detailed report construction and memory hashing.

## Keyboard and display

| Host key | Action |
| --- | --- |
| Escape | Send raw `95` |
| F1 | Send raw `93` |
| F2 | Send raw `97` |
| Enter | Send CR (`0d`) |
| Backspace | Send `08` |
| Tab | Send `09` |
| Ctrl-V followed by two hexadecimal digits | Send a raw key byte |
| Ctrl-] or Ctrl-C | Exit and restore terminal settings |

Interactive SIGINT, SIGTERM, and SIGHUP request shutdown through terminal cleanup,
then return exit status 128 + signal number after any requested report. Keyboard
Ctrl-C/Ctrl-] retain successful `user_exit` behavior. SIGKILL cannot run cleanup.
Host output is nonblocking with a bounded wait, including best-effort screen
restoration if the terminal stops accepting output.

Guest software determines the meaning of each byte. Historical physical key
identities are not fully established. Unknown host key sequences are consumed
atomically rather than leaking their suffixes into guest input.

Override host mappings with a JSON file:

```sh
./target/release/rocc-emulator boot /path/to/disk.img --interactive \
  --keymap keymap-example.json
```

Supported names: Enter, Backspace, Tab, Escape, Up, Down, Left, Right, Home, End,
Insert, Delete, PageUp, PageDown, BackTab, and F1 through F12. Values are exactly
two hexadecimal digits. Omitted keys retain defaults. Host exit and raw-entry
shortcuts remain reserved. Modified function-key sequences are not interpreted
as their unmodified counterparts.

The terminal model handles cursor positioning, field fills, selected clear
sequences, attributes, and cursor save/restore. Unknown display controls remain
diagnostic events. Terminal semantics are provisional; guest bytes are never
passed through as host ANSI commands.

## Tests

Build the release executable before running the Python test harnesses:

```sh
cargo test --locked
cargo build --release --locked
python3 tests/cli.py
python3 tests/regression.py --cpu
python3 tests/reference_tests.py
python3 tests/generate_reference.py --check
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
```

The CLI tests use synthetic temporary images. The CPU regression compares
8,262 frozen instruction cases and 610 independently generated cases, including
64 multi-instruction programs. See [reference provenance](tests/REFERENCE.md);
these corpora establish model consistency, not hardware fidelity. CI runs the
suites on Rust 1.87 and stable. Tests
require no external disk image or private credentials.

## Source layout

- [CPU instruction-set reference](docs/cpu-isa.md): instruction formats,
  addressing, arithmetic, interrupts, and recovered extensions.
- [Machine architecture](docs/machine-architecture.md): memory map, controllers,
  terminal protocol, bootstrap assumptions, and compatibility limits.

- `src/cpu.rs`, `src/memory.rs`: instructions and banked memory.
- `src/devices.rs`: controller behavior and changed-block disk overlay.
- `src/overlay.rs`: persistent overlay journal and recovery.
- `src/terminal.rs`, `src/screen.rs`: terminal protocol and display state.
- `src/interactive.rs`: host keyboard, rendering, and terminal lifecycle.
- `src/config.rs`, `src/fault.rs`: validated settings and typed faults.
- `src/machine.rs`, `src/main.rs`: shared instruction clock, orchestration and CLI.
- `src/report.rs`, `src/artifact.rs`: streaming reports and durable publication.
- `src/signals.rs`: scoped interactive shutdown handlers.

See [performance probes](docs/performance.md) for reproducible measurements and
[machine API boundaries](docs/machine-architecture.md#11-rust-api-boundaries) for
Rust library migration notes.
