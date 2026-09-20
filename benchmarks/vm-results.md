# VM recovery refactor: 2026-09-19

Baseline: commit `638f458`, rebuilt with the same release profile (`opt-level=3`, fat LTO, one codegen unit).
Platform: `Windows-11-10.0.26200-SP0`. Three independent runs per binary and case; medians below.
The before/after driver runs binaries sequentially, alternates order, and includes startup, compilation and I/O. Builds and tests were stopped during measurement.

## Changes

- Choicepoints share immutable stack prefixes instead of cloning entire vectors, including nested recovery handlers. Ordinary stacks reuse eight-element chunks; handlers and labels use one-element chunks to avoid overallocating short-lived large records. Snapshots copy only heads and positions. Unique pops move values, and chunk destruction is iterative.
- Array/object iteration stores the original collection and next index, with at most one pending choicepoint per iterator. It no longer clones every value/key or eagerly creates all continuations.
- A `PATH` const generic removes operand-path maintenance when the program cannot observe paths. Published path bytecode and exported REPL closures conservatively enable tracking, including closures used by future entries.
- Decimal literals share digits through `Rc`; integer-to-float conversion avoids string formatting/parsing for up to 19 integer digits. Literal precision, exact comparison, signed zero and the existing fallback conversion remain intact.

`Value::Decimal` now contains `Rc<Decimal>` instead of `Box<Decimal>`; callers constructing the enum variant directly must adapt. `Value::decimal` is unchanged.
Saved operand lookup traverses chunk links for older operands; this is a tradeoff compared with direct vector indexing. Binding paths still use the existing public `SlotValue` representation.

## Before/after results

| Case | Before (ms) | After (ms) | Speedup |
|---|---:|---:|---:|
| add | 1716.76 | 1082.47 | 1.59x |
| complex | 26.63 | 27.17 | 0.98x |
| empty | 461.15 | 267.09 | 1.73x |
| fibo | 2880.54 | 2328.80 | 1.24x |
| last | 555.31 | 359.69 | 1.54x |
| math | 773.25 | 636.95 | 1.21x |
| try-catch | 2238.78 | 2225.88 | 1.01x |
| iterate-all | 212.62 | 111.06 | 1.91x |
| iterate-first | 195.00 | 44.88 | 4.35x |
| nested-recovery | 127.08 | 27.31 | 4.65x |
| paths | 65.08 | 54.63 | 1.19x |

All 11 workloads produced identical stdout, stderr and exit codes before/after on every run.
The short `complex` workload and `try-catch` are effectively unchanged; small percentage differences should not be interpreted as significant.
The early iteration and nested recovery cases include fixed startup costs, so these are end-to-end gains, not isolated VM timings.

## Validation

- Seven new regression tests cover persistent stack ownership/branching, bounded iterator continuations, generator/error/path behavior, future REPL closure paths, and exact numeric conversion.
- 92 tests pass when excluding the three pre-existing failing test functions (8 library, 48 integration, 36 data/compiler/parser tests).
- Unfiltered `cargo test` still fails in the upstream jq test function: 19 of its 550 cases fail. Separate unit execution also finds two fixture tests failing on Windows CRLF line endings. Both failure groups were reproduced by independently building the original source with the same fixture bytes.
- No compiler rewrite/optimization pass was changed.

See [benchmark commands](README.md). Raw timings, binary hashes and output hashes are saved in `target/vm-final.json`; test logs are in `target/vm-cargo-test.log`, `target/vm-filtered-tests.log`, `target/vm-baseline-unit.log` and `target/vm-baseline-upstream.log`.

## Reference tools

The existing Go driver was run with `-runs 1 -timeout 10s`. These tools run concurrently, so their wall times should not be mixed with the sequential before/after medians above. All times below are milliseconds.

| Case | jq | jaq | gojq | 1q |
|---|---:|---:|---:|---:|
| add | 891.15 | 519.41 | 370.65 | 1213.34 |
| complex | 3.28 | 13.21 | 3.80 | 22.68 |
| empty | 398.28 | 1486.10 | 424.24 | 301.57 |
| fibo | 1401.24 | 2597.98 | 907.13 | 2515.79 |
| last | 520.85 | 111.76 | 540.69 | 394.85 |
| math | 409.53 | 471.13 | 475.75 | 661.57 |
| try-catch | 1897.07 | 1243.77 | 1066.69 | 2431.29 |
| iterate-all | 75.08 | 62.93 | 86.00 | 126.88 |
| iterate-first | 24.83 | 14.11 | 42.79 | 42.79 |
| nested-recovery | 2.67 | 3.20 | 3.20 | 24.35 |
| paths | 31.53 | 22.02 | 22.74 | 61.44 |

The Go driver still reports the pre-existing textual mismatch for `complex` (Windows newline differences and differences between tool renderings); the 1q before/after output is identical. The four new VM cases match the reference outputs. There is still substantial room to improve performance relative to the other engines, especially compilation/startup and recursion.

A final differential run of all 550 upstream cases found identical stdout and exit status between the original and final release binaries (`target/vm-differential.json`).
