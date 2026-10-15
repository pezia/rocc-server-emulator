---
layout: default
title: The ROCC machine and its emulator
---

# Bringing a ROCC system back to the screen

This project is an experimental Rust emulator for hardware provisionally
identified as the **ROCC 2800 Classic** family. It reconstructs a CPU, banked
memory, disk controller, interrupts, and a 24×80 terminal so that original guest
software can execute its own instructions. The exact machine model and processor
revision remain unresolved.

[The machine](#the-machine) · [Running original software](#running-original-software) ·
[Historical sources](#historical-sources) · [Try the emulator](#try-the-emulator)

## What is a ROCC computer?

ROCC's Classic systems were business minicomputers descended from the 16-bit
Data General Nova architecture. They served multiple operators at terminals,
supporting data entry and business processing. The software environment evolved
from releases known as “Loads” into the Workstation Management System (WMS).
The [Michael Aldrich Archive's technical history](https://www.aldricharchive.co.uk/a-technical-history-of-rocc-computers)
traces that development and describes the 1986–87 Classic range, including the
2825 and 2835 in the compact “Miniboy” cabinet.

That history describes a family of machines; it does not establish the precise
configuration emulated here. Our hardware documentation records which behaviors
are inferred and which details remain open.

## The machine

These spaces are reserved for photographs supplied by the machine's owner.
They will show the physical computer and its terminal, with identification and
credits attached to each photograph.

{% include gallery.html items=site.data.gallery.machine %}

## Running original software

The emulator loads a user-supplied disk image and executes guest code. That code
performs subsequent loading, interrupt handling, scheduling, and terminal I/O
through the modeled hardware. The host renders the terminal and forwards key
events to the guest.

The gallery below is reserved for actual captures of a real guest system running
in the emulator. Screenshots have not been supplied yet; each capture will identify
the software release and explain what the screen demonstrates.

{% include gallery.html items=site.data.gallery.screenshots %}

Hardware behavior is still being reconstructed. Unsupported operations stop with
diagnostics, and compatibility with a particular operating-system release needs
separate validation. Public tests use synthetic instructions and disk images;
they do not establish that an entire historical system works.

## Historical sources

- [Michael Aldrich Archive — A Technical History of ROCC Computers](https://www.aldricharchive.co.uk/a-technical-history-of-rocc-computers):
  hardware generations, Nova ancestry, terminals, and the Classic/WMS family.
- [Michael Aldrich Archive — ROCC Technical Documents](https://www.aldricharchive.co.uk/rocc-technical-documents):
  a catalogue of preserved manuals and technical material, listed as a
  non-digitised archive rather than downloadable emulator documentation.
- [Centre for Computing History — ROCC Server](https://www.computinghistory.org.uk/det/50187/ROCC-Server/):
  a museum collection record connecting ROCC hardware with Viewdata and an early
  home-shopping service. This is historical context, not an identification of
  the machine emulated here.

## Try the emulator

Build the project with Rust 1.87 or later and a C linker, then supply a compatible
disk image:

```sh
cargo build --release --locked
./target/release/rocc-emulator boot /path/to/disk.img --interactive
```

Interactive mode requires a POSIX terminal at least 80 columns wide and 24 rows
high. The image is opened read-only; guest writes remain in a private in-memory
overlay for the current run. Operating systems, disk images, and application
data are not included.

- [Build instructions and keyboard controls](https://github.com/pezia/rocc-server-emulator/blob/main/README.md)
- [Machine architecture and compatibility limits](https://github.com/pezia/rocc-server-emulator/blob/main/docs/machine-architecture.md)
- [CPU instruction-set reference](https://github.com/pezia/rocc-server-emulator/blob/main/docs/cpu-isa.md)
- [Source repository](https://github.com/pezia/rocc-server-emulator)
