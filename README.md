# ROCC emulator

An experimental Rust emulator for inferred ROCC hardware: CPU, banked memory,
disk controller, interrupts, and a 24×80 terminal. Hardware behavior is still
being reconstructed; unsupported instructions and controller operations stop
execution with diagnostics.

No operating system, disk images, application data, or credentials are included.
Supply your own compatible image explicitly.

## GitHub Pages

The [public introduction](docs/index.md) explains the ROCC machine, links to
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

The input image is read-only. Guest writes stay in a private in-memory overlay;
there is no overlay-resume feature. Guest time is instruction-based, not
cycle-accurate.

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
```

The CLI tests use synthetic temporary images. The CPU regression compares
8,262 generated instruction cases against frozen reference results. Tests
require no external disk image or private credentials.

## Source layout

- [CPU instruction-set reference](docs/cpu-isa.md): instruction formats,
  addressing, arithmetic, interrupts, and recovered extensions.
- [Machine architecture](docs/machine-architecture.md): memory map, controllers,
  terminal protocol, bootstrap assumptions, and compatibility limits.

- `src/cpu.rs`, `src/memory.rs`: instructions and banked memory.
- `src/devices.rs`: controller behavior and private disk overlay.
- `src/terminal.rs`, `src/screen.rs`: terminal protocol and display state.
- `src/interactive.rs`: host keyboard, rendering, and terminal lifecycle.
- `src/machine.rs`, `src/main.rs`: orchestration, CLI, and reports.
