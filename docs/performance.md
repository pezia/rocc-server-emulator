# Reproducible synthetic performance probes

Build and run from the repository root:

```sh
cargo build --release --locked
cargo build --release --locked --example performance
python3 tests/performance.py
```

The driver uses temporary synthetic images and 120-second subprocess timeouts.
It reports host identity, operation timing, wall time, and peak RSS when GNU time
is available. Individual probes can be run as
`cargo run --release --example performance -- CASE ITERATIONS`, where CASE is
`instructions`, `terminal-tail`, or `report`. Reports go to an I/O sink so the
report probe measures serialization/hash/allocation rather than filesystem speed.
The disk blocks are prepared in memory; durable guest-write sync latency is not
part of instruction or report throughput. The replay probe separately includes
startup and compaction filesystem overhead.

A representative run on Linux x86_64/glibc 2.43, Rust 1.96.1, release profile:

| Probe | Before | After | Peak RSS before / after |
| --- | --- | --- | --- |
| 10 million simple JMP instructions | 0.13549 s | 0.14951 s | 2228 / 2456 KiB |
| 10,000 one-byte outputs after 1 MiB history fills | 0.13785 s | 0.000607 s | 8884 / 4756 KiB |
| Export 10,000 changed disk blocks | 0.02748 s | 0.00634 s | 32052 / 5668 KiB |

The simple instruction loop is about 10% slower in this sample with checked memory
mapping. Output retention avoids shifting a full MiB per byte; typed diagnostics
also avoid JSON allocations during successful controller operations. Streaming
report export avoids constructing all changed blocks and packets as a JSON tree.
Peak RSS includes setup; operation timing excludes setup. Results are single-host
measurements, not timing guarantees or correctness assertions. Repeat measurements
on the intended host before choosing instruction budgets or resource settings.

The baseline was reconstructed from pre-implementation revision `98f5beb` in a
separate temporary source archive, using the same probes with only old API call
adaptations (infallible Cpu construction, memory set, controller Cpu argument,
and explicit old JSON report serialization). The initial preflight loop measured
0.13893 s for 10 million instructions, consistent with that reconstructed baseline.

A 20,000-record overlay touching eight blocks shrank from 5,920,048 to 2,416 bytes.
One-step boot wall time fell from 0.01917 s to 0.00176 s. Compaction itself took
0.01891 s. These short durations include process startup and filesystem cache
noise. The original journal and base image were preserved.

Do not add machine-specific performance assertions to the correctness suite.
