# ROCC CPU instruction set

Companion: [Machine architecture](machine-architecture.md).

## Scope and confidence

This is a complete reference to the instruction families and exact ROCC extension
encodings currently understood by this project, NOT a complete vendor ISA manual.
Execution tests cover the implemented model; they do not verify every hardware
variant, exception case, processor revision, or peripheral.

Evidence labels used below:
| Label | Meaning |
| --- | --- |
| BASE | Nova-family behavior implemented in Rust and exercised by synthetic fixtures. |
| RECOVERED | Behavior inferred from software execution dependencies. |
| MODEL | Emulator choice or limit, not established physical behavior. |
| OPEN | Insufficient evidence; no hardware semantics assigned. |

The working target is ROCC 2800 Classic; exact model/processor revision is OPEN.
Instruction names BMOV/LDB/STB below are descriptive project names, not recovered
vendor assembler spellings. Guest runtime bytecode is a separate execution layer
and must not be decoded as native CPU instructions.

## Notation

Hexadecimal values have prefix 0x; decimal numbers are otherwise used. Device
numbers have prefix 0o and are octal. A RAM address names a 16-bit WORD, not a byte.
Bit 15 is the most significant bit and bit 0 the least significant bit. This is the
opposite bit-numbering direction from some DG documents. M[a] means a memory word;
S/D are source/destination accumulators; C is the one-bit carry. Instruction words
and stored memory words use big-endian byte order in the recovered representation.

## Programmer-visible state

| State | Description |
| --- | --- |
| AC0–AC3 | Four 16-bit accumulators; AC2 and AC3 also serve as address bases. |
| PC | Word-addressed program counter; 16 bits in the extended profile. |
| C | One carry bit, included in 17-bit rotations. |
| IRQ | Enable, deferred enable, and interrupt mask. |
| BANK | External memory-controller selector. |

There are no modeled independent zero/negative/overflow flags. ALU instructions
can skip using their candidate result and carry. No dedicated hardware stack,
privilege mode, floating-point state or multiply/divide unit has been recovered.
A normal instruction advances PC by 1; a successful skip advances it by 2. PC/EA
arithmetic wraps to the selected logical address width. Memory words wrap 16 bits.
Cycle counts and real-time instruction timing are OPEN.

## 1. Instruction formats and decode precedence

Every decoded instruction occupies one 16-bit word. Following words may be data
or software operands; do not classify all words of an image as instructions.

Memory-reference family (word <0x6000):

| Bits | Field |
| --- | --- |
| 15–11 | Operation (0–11) |
| 10 | Indirect flag `I` |
| 9–8 | Addressing mode |
| 7–0 | Displacement `disp8` |

```text
word = (operation << 11) | (I << 10) | (mode << 8) | disp8
```

I/O family (0x6000 <= word <0x8000):

| Bits | Field |
| --- | --- |
| 15–13 | Fixed `011` |
| 12–11 | Accumulator |
| 10–8 | Operation |
| 7–6 | Pulse |
| 5–0 | Device |

```text
word = 0x6000 | (AC << 11) | (operation << 8) | (pulse << 6) | device
```

ALU family (word >=0x8000):

| Bits | Field |
| --- | --- |
| 15 | Fixed `1` |
| 14–13 | Source accumulator |
| 12–11 | Destination accumulator |
| 10–8 | Operation |
| 7–6 | Shift |
| 5–4 | Carry selection |
| 3 | No-load flag `NL` |
| 2–0 | Skip condition |

```text
word = 0x8000 | (S << 13) | (D << 11) | (operation << 8)
              | (shift << 6) | (carry << 4) | (NL << 3) | skip
```

In the recovered boot CPU, exact words 0x8208/0xABC9/0xBBC9/0xFBC9 override
classic ALU interpretation. Other ALU words with NL=1 and skip=0 stop as unknown
ROCC extensions. Remaining words use the base decoder, including ordinary
no-load comparisons. Additional extensions in that space have NOT been excluded.
The format partition covers all 65,536 words: 24,576 memory-family encodings,
8,192 I/O-family encodings and 32,768 ALU-family encodings. Format coverage is not
hardware support: 2,048 ALU words have NL=1/skip=0, and only 0x8208 of those is
currently assigned a recovered instruction. The other three recovered byte
operations override words in a different part of the ALU space.
The execution decoder in `src/cpu.rs` applies these overrides before the base
ALU decoder.

## 2. Memory-reference instructions [BASE]

| Code | Base word | Mnemonic | Effect after effective-address calculation |
| --- | --- | --- | --- |
| 0 | `0x0000` | `JMP EA` | `PC = EA` |
| 1 | `0x0800` | `JSR EA` | `AC3 = (old PC + 1) & address_mask; PC = EA` |
| 2 | `0x1000` | `ISZ EA` | Increment memory modulo 65536; skip if zero. |
| 3 | `0x1800` | `DSZ EA` | Decrement memory modulo 65536; skip if zero. |
| 4–7 | `0x2000`–`0x3800` | `LDA ACn,EA` | `ACn = M[EA]`; `n = operation & 3`. |
| 8–11 | `0x4000`–`0x5800` | `STA ACn,EA` | `M[EA] = ACn`; `n = operation & 3`. |

LDA/STA base words increase by `0x0800` per accumulator.

These instructions preserve C. JSR overwrites AC3 even when AC3 participated in
EA calculation: EA is calculated first. Return is software, e.g. JMP 0(AC3).
Nested calls require software preservation of return addresses.

Address modes:

```text
  mode0: EA = zero_extend(disp8), range 0..255 (page zero).
  mode1: EA = instruction_PC + sign_extend(disp8).
  mode2: EA = AC2 + sign_extend(disp8).
  mode3: EA = AC3 + sign_extend(disp8).
```

Relative displacement range is -128..127. The PC base is the CURRENT instruction,
not the already incremented PC. Mask the sum to 15 or 16 bits as configured.

If I=1, dereference the computed address. Before using a pointer in RAM 16..23
(0o20..0o27), increment that pointer word. For RAM 24..31 (0o30..0o37), decrement
it. Use the updated value. Autoindex happens only on an indirect access, not on
an ordinary LDA/STA of those addresses.

Recovered 16-bit profile [RECOVERED/MODEL]: one full 16-bit pointer dereference;
bit 15 is address data, not a continuation flag. This allows the original code to
reach 0x8000..0xFFFF. Runtime selection between 15/16-bit modes is OPEN.

Classic 15-bit diagnostic profile [BASE]: pointer bit 15 requests another level
of indirection; remaining 15 bits give the next address. Continuation is tested
from the pointer BEFORE an autoindex modification. The emulator limits this to
256 dereferences [MODEL]. Autoindex changes preceding a later fault remain visible.

## 3. ALU operations [BASE]

Execution order: choose temporary carry; perform arithmetic; apply shift/swap;
evaluate skip; optionally write destination and carry. Source/destination values
are read before writeback, so S=D is permitted.

Use this exact 17-bit formulation to avoid conventional carry/borrow assumptions:

```text
  k = selected temporary carry
  v = (k<<16) | AC[S]
  d = AC[D]
```

| Operation | Code | Transformation of `v` |
| --- | --- | --- |
| COM | 0 | `v XOR 0xFFFF` |
| NEG | 1 | `(v XOR 0xFFFF) + 1` |
| MOV | 2 | `v` |
| INC | 3 | `v + 1` |
| ADC | 4 | `(v XOR 0xFFFF) + d` |
| SUB | 5 | `(v XOR 0xFFFF) + d + 1` |
| ADD | 6 | `v + d` |
| AND | 7 | `v AND (d OR 0x10000)` |

Mask v to 0x1FFFF after arithmetic. Thus SUB computes destination minus source;
ADC means add complement, NOT the usual modern "add with carry" instruction.
The carry is carried through/toggled by the 17-bit calculation. It is not simply
an unsigned-overflow flag freshly overwritten after every operation. The formula
above defines edge cases such as NEG 0 and SUB with equal operands precisely.

Carry field (bits 5..4), selected BEFORE arithmetic:

```text
  0  no suffix  k = old C
  1  Z          k =0
  2  O          k =1
  3  C          k =1-old C
```

Shift field (bits 7..6), applied AFTER arithmetic:

```text
  0  no suffix  no transformation
  1  L          rotate 17-bit v left 1: ((v<<1)|(v>>16))&0x1FFFF
  2  R          rotate 17-bit v right 1: (v>>1)|((v&1)<<16)
  3  S          swap low/high bytes of low 16 bits; keep bit 16 unchanged
```

L/R rotate THROUGH carry; they are not ordinary 16-bit logical shifts. Carry
selection makes useful logical shifts possible. Repeating ADDZL of an AC to
itself multiplies by 4 per instruction, not 2.

Set r=v&0xFFFF and c=(v>>16)&1 after the shift. Skip field:

```text
  0  (none)  false
  1  SKP     true
  2  SZC     c==0
  3  SNC     c==1
  4  SZR     r==0
  5  SNR     r!=0
  6  SEZ     c==0 OR r==0
  7  SBN     c==1 AND r!=0
```

NL=0 writes AC[D]=r and C=c. NL=1 (suffix#) preserves BOTH destination and C,
while the skip still uses r/c. The ROCC overrides/reserved space in section 1
must be checked before this rule. In particular, 0x8208 is not a harmless NOP.
Assembler-style suffix order in this project is carry, shift, then #; e.g.
MOVZL, ANDS, SUBZ#. Hardware assembler syntax has not been recovered.

## 4. General I/O instructions [BASE format, device-specific semantics]

| Operation | Code | Base | Transfer or effect |
| --- | --- | --- | --- |
| NIO | 0 | `0x6000` | No register transfer; issue pulse/control. |
| DIA | 1 | `0x6100` | Device A → selected AC. |
| DOA | 2 | `0x6200` | Selected AC → device A. |
| DIB | 3 | `0x6300` | Device B → selected AC. |
| DOB | 4 | `0x6400` | Selected AC → device B. |
| DIC | 5 | `0x6500` | Device C → selected AC. |
| DOC | 6 | `0x6600` | Selected AC → device C. |
| SKP | 7 | `0x6700` | Conditional skip from device status. |

Add AC selection, pulse and device fields to the base. I/O preserves CPU carry.
A/B/C are controller transfer registers; they are distinct from CPU accumulators
and the CPU carry. Device 0o77 is CPU control, with special operation meanings.

Pulse field for operations 0..6:

```text
  0  no suffix  no pulse
  1  S          start/set convention
  2  C          clear convention
  3  P          peripheral-specific pulse
```

These are conventions, not universal peripheral behavior. E.g. terminal DICC
consumes an event, while disk DOCS starts a transfer. Only enumerated combinations
are implemented. There is no generic success response for unsupported devices.

For SKP, pulse bits choose a condition instead:

```text
  0  SKP   skip if BUSY=1
  1  SKPS  skip if BUSY=0
  2  SKPC  skip if DONE=1
  3  SKPP  skip if DONE=0
```

The SKP/SKPS/SKPC/SKPP notation here uses generic pulse suffixes rather than
DG condition-name aliases. Use the pulse numbers and conditions above.

## 5. CPU device 0o77 and interrupts [BASE/RECOVERED/MODEL]

Canonical AC0 encodings (other AC fields select the transferred accumulator):

```text
  0x603F NIO  0,0o77  no transfer; no pulse change
  0x607F NIOS 0,0o77  ION: enable, recognize IRQ after one following instruction
  0x60BF NIOC 0,0o77  IOF: disable, cancel deferred enable
  0x633F DIB  0,0o77  read highest-priority modeled pending device number, or 0
  0x643F DOB  0,0o77  write interrupt-disable mask (MSKO-style)
  0x653F DIC  0,0o77  modeled reset: disk reset, disable IRQs, clear mask;
                       preserve selected accumulator
  0x65BF DICC 0,0o77  same reset, with explicit disable pulse
  0x663F DOC  0,0o77  HALT
```

Supported CPU transfers: NIO/DIB/DOB/DIC, pulses 0/1/2. Pulse 1 enables with delay 1;
pulse 2 disables with delay 0. DIC's reset is modeled only for disk; reset effects
on other devices are OPEN. CPU DIA/front-panel switch reading, DOA and pulse 3
controls are unsupported. In particular, no 15/16-bit mode-switch opcode is known.

HALT matches (word&0xE7FF)==0x663F:0x663F,0x6E3F,0x763F,0x7E3F all halt; the
AC field is ignored. Pulse variants are not matched. The emulator records PC+1
when halting; this is a trace convention, not proof of a physical panel PC value.

CPU SKP pulses 0..3 test IRQ-enabled, IRQ-disabled, false, true respectively.
The latter two are a modeled absence of a power-fail condition, not recovered
power-failure circuitry. DIB pending-device lookup does not itself clear device
completion: the original interrupt handler acknowledges the peripheral.

On IRQ recognition: M[0]=interrupted PC; PC=M[1]; disable IRQs; preserve AC0..3
and carry. Original code saves/restores them in software. Current priority is
 disk 0o33, then terminal 0o50; disable mask bits are 0x2000 and 0x8000 respectively.
This wiring is inferred from ROCC startup, not copied from a DG device map.
The emulator executes the first handler instruction in the same step as IRQ
entry. The report keeps a bounded instruction tail; it is not a full IRQ trace.

## 6. ROCC extension 0x8208 — forward bank-aware block copy [RECOVERED]

Inputs: AC0=destination word address; AC1=source word address; AC3=word count.
AC2 and carry are preserved. Source local bank is M[2]>>8; destination local bank
is the selected memory-controller bank. Shared words ignore bank selection.

For each n from 0 to count-1, in increasing order:

```text
  source physical word = AC1+n, using the RAM 2 bank for local-window addresses
  destination physical word = AC0+n, using selected bank for local addresses
  copy source to destination, observing earlier writes to the SAME physical word
```

Then AC0+=count and AC1+=count modulo 65536, AC3=0, PC+=1.
This is forward copy, not memmove: overlap may propagate a fill. Equal logical
addresses in different banks are not overlapping physical memory.

Zero count copies nothing and preserves pointers. A range ending exactly at 65536
is accepted and its resulting pointer wraps to 0; a range beyond 65536 is rejected
by the emulator. Complete source/range validation precedes writes/register updates
[MODEL atomic-failure policy]. Global-only tasks cannot write the local window;
an explicitly banked source may still be read. Unsupported source banks stop.
The low byte of RAM 2 is ignored by this copy; its hardware meaning remains OPEN.

Earlier loader-v1 preserves registers and uses selected-bank source memory. That
historical hypothesis is retained for reproducibility and is NOT this reference.

## 7. ROCC byte-field operations [RECOVERED]

| Word | Project name | Effect |
| --- | --- | --- |
| `0xABC9` | `LDB field → AC0` | Zero-extend the selected byte into AC0. |
| `0xBBC9` | `LDB field → AC1` | Zero-extend the selected byte into AC1. |
| `0xFBC9` | `STB AC1 → field` | Store `AC1 & 0xFF`; preserve the other byte. |

All three use a descriptor relative to current AC2:

```text
  low  = M[(AC2-68)&0xFFFF]
  high = M[(AC2-67)&0xFFFF]       only 0 or 1 currently accepted
  byte_pointer = low | (high<<16)
  word_address = byte_pointer>>1
  shift = 8 if byte_pointer is even, else 0
```

Loads: ACtarget = (M[word_address]>>shift)&0xFF.
Store: M[word_address] = (old_word & ~(0xFF<<shift)) | ((AC1&0xFF)<<shift).
Addresses use the currently selected bank, NOT RAM 2's copy-source bank.
Descriptor and all other accumulators remain unchanged; C remains unchanged;
PC advances 1, with NO skip and no descriptor increment. The 17-bit byte pointer
covers all 128 KiB of the logical address space. Unsupported descriptor highbits,
uninitialized memory and global-only local references stop without partial edits.

The implementation supports only the exact recovered words. Other destination registers,
descriptor locations, byte-order modes or related encodings are OPEN.

## 8. Unknown instruction space and non-claims

```text
  0xA208 : candidate further cross-bank copy variant; unsupported.
  0xC218 : candidate extension; operation unresolved; unsupported.
  Other NL=1,skip=0 words: stop as unsupported_rocc_instruction.
  Other possible aliases/extensions: not exhaustively identified.
  MUL/DIV, FP, Nova3/4 stack/trap/byte extensions: no ROCC contract recovered.
  Timing, interruptibility/restart of block moves, bus errors, invalid descriptors
  on physical hardware, privilege, diagnostics and front-panel behavior: OPEN.
```

The current fail-closed rules are research safeguards, not claims that hardware
would fault in the same way. Base-decoder acceptance is not proof that a word is
valid ROCC code. Do not substitute later Nova extensions merely because a bit
pattern matches them, or decode guest runtime bytecode as this ISA.

## 9. Implementation and validation

The Rust implementation is authoritative for the emulator behavior described here:

- [CPU](../src/cpu.rs): decoding, addressing, arithmetic, extensions, interrupts.
- [Memory](../src/memory.rs): bank mapping and unknown-cell tracking.
- [Devices](../src/devices.rs): controller transfers and CPU control operations.
- [Terminal](../src/terminal.rs): terminal event protocol.
- [CPU regression](../tests/regression.py): 8,262 synthetic instruction cases.
- [Machine architecture](machine-architecture.md): memory and device contracts.

From the emulator repository root:

```sh
cargo test --locked
cargo build --release --locked
python3 tests/regression.py --cpu
```

Unit tests cover arithmetic edge cases, autoindexing, interrupt timing, block-copy
overlap and failure atomicity, byte fields, bank isolation, and controller behavior.
Frozen synthetic fixtures check state and memory results. These checks establish
implementation consistency; they do not substitute for a vendor specification.


Instruction-time orchestration is owned by `Machine::execute_step`, shared by
boot execution and fixture execution. `Cpu::step` forwards to that method;
`Cpu::execute` alone performs no terminal tick or interrupt entry. The independent
base-ISA corpus and its limitations are documented in
[reference provenance](../tests/REFERENCE.md). The original 8,262 frozen cases
remain unchanged; 610 new cases include 64 short native programs. Controller
integration programs separately cover IRQ delay/masks, bank selection, DMA and
terminal event acknowledgement.
