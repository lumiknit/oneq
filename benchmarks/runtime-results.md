# Runtime allocation optimization: 2026-09-20

Baseline: `ba9c1a5`, built with the existing release profile. Windows x64;
three independent CLI processes per binary/case, sequential execution with
alternating before/after order. No builds or tests ran during final timings.
Times include startup, compilation, input parsing and output rendering.

## Changes

- Builtin argument lists of up to three values use a stack buffer instead of
  allocating a `Vec` on each invocation. Larger argument lists retain a heap
  fallback. Infix argument ordering and consuming array/object addition remain
  unchanged. `BuiltinImpl::OwnedArgs` now accepts `&mut [Value]` instead of
  `Vec<Value>`; consumers take values out of that slice.
- Return, tail-call and backtrack paths reclaim uniquely owned lexical frames.
  A per-VM pool holds at most 32 empty frames, retaining their slot capacity.
  Both the frame allocation and its slot allocation can be reused. Frames
  retained by closures, return addresses or choices cannot enter the pool.
  The pool itself is not captured in choice points.
- Added a standalone allocation probe; production builds contain no counters.
- Corrected the Go driver's startup-timing comment and `-runs` help: repetitions
  are inputs within one process, not independent process samples.

No compiler, inlining, instruction selection, or builtin jq definitions changed.

## Timing

| Case | Before (ms) | After (ms) | Speedup |
|---|---:|---:|---:|
| add | 1053.8 | 980.8 | 1.07x |
| math | 610.0 | 510.5 | 1.20x |
| fibo | 2329.4 | 1926.4 | 1.21x |
| complex | 24.8 | 26.0 | 0.95x |
| empty | 265.6 | 271.1 | 0.98x |
| last | 352.9 | 358.5 | 0.98x |
| try-catch | 2266.3 | 2270.8 | 1.00x |
| iterate-all | 109.6 | 99.1 | 1.11x |
| iterate-first | 40.5 | 40.0 | 1.01x |
| nested-recovery | 23.1 | 23.6 | 0.98x |
| paths | 54.4 | 52.5 | 1.04x |

All 11 cases have byte-identical stdout/stderr and successful exit codes in
every before/after sample. Small differences in the other cases are not evidence
of a significant improvement or regression with only three samples.
Raw samples and binary hashes: `target/runtime-final.json`.

## Allocation evidence

The probe excludes compilation, parsing and rendering, and consumes the original
case's input sequence once. Counts include successful reallocations.

| Case | Baseline requests | Stack arguments only | Plus frame reuse |
|---|---:|---:|---:|
| add | 7,340,088 | 6,291,511 | 4,194,360 |
| math | 5,390,029 | 2,570,028 | 1,310,038 |
| fibo | 29,090,391 | 18,232,695 | 9,546,855 |

| Case | Requested bytes before | Requested bytes after | Peak extra live bytes before | Peak extra live bytes after |
|---|---:|---:|---:|---:|
| add | 973,085,856 | 813,702,432 | 192,945,905 | 192,946,033 |
| math | 550,553,640 | 354,474,968 | 402,528 | 403,584 |
| fibo | 3,559,127,664 | 2,586,318,672 | 12,926 | 17,598 |

Reusing frames reduces allocation traffic while slightly increasing retained
live heap. Requested bytes count the entire target size of each reallocation;
they are not bytes physically copied or transferred to DRAM.

### Interpreting each workload

`add` is `[range(.) | [.]] | add` with input 1,048,576. This creates over a million
small arrays before concatenating them. The original implementation already
moves the fold accumulator out on `FoldLoad`, releases evaluation-only aliases,
and uses `Rc::make_mut` for addition. There is no evidence of quadratic copying
in this workload; the original cumulative allocation volume is below 1 GB.
Snapshots retained for actual generator alternatives still correctly force COW.

As a decomposition probe on the original runtime with the same input:

- `[range(.)]`: 31 allocation/reallocation requests.
- `[range(.) | [.]]`: 2,097,183 requests.
- `reduce range(.) as $x (0; . + $x)`: 5,242,892 requests.

Thus range does not allocate a heap object per emitted number; small-array
construction and the generic binding/operand machinery are substantial costs.
The remaining array materialization explains why `add` improves less than the
other two workloads. This benchmark also includes rendering a million elements
in its final array in the CLI timing, unlike the allocation probe.

`math` evaluates the inner expression 200,000 times. Binary arithmetic repeatedly
allocated argument vectors; `$i`, `$j`, `$x` and fold bindings also allocated
lexical regions. The two changes remove about 76% of allocation requests without
changing `range`, collection semantics, or floating-point operations.

`fibo` executes the naive recursion for inputs 20, 22, 24, 26, 28 and 30.
It is not tail-recursive: addition still needs the results of both calls.
Its small peak live heap but nearly 29.1 million original allocation requests
exposes rapid temporary allocation/destruction. Conditional bytecode uses
`JumpFalse`, not a choice point for each recursive call. Argument storage and
activation allocation are therefore more directly relevant here than shrinking
recovery state. Persistent operand/call stack chunk management and dispatch
remain candidates for further profiling; no CPU sampling or hardware memory
traffic measurement was performed in this change.

## Why begin/end instructions differ

These delimit dynamic execution regions, rather than ordinary branch targets.

| Region | State and completion behavior |
|---|---|
| label | Saves a break recovery context. Break removes inner choices/collections, restores the context and backtracks; EndLabel removes lexical visibility. |
| try | Saves exception recovery state. EndTry closes the active lexical handler; saved generator continuations retain their own handlers. |
| alternative | Stores a shared success flag. The fallback choice runs only if no truthy left result was produced, even across backtracking. |
| fold | Stores a shared accumulator that intentionally survives source backtracking. EndFold produces the final accumulator. |
| collect | Keeps an append-only accumulator outside snapshots. A saved exhaustion continuation reaches EndCollect after all inner outputs have been collected. |

ChoicePoint shares persistent stack prefixes in O(1), rather than copying every
stack element. Input/frame/operands, call continuation, recovery stacks and path
state describe different things. Removing apparently empty or overlapping fields
requires preserving closures, nested paths and backtracking semantics. Snapshot
record copying and reference-count updates remain costs; this work does not
claim that the record is already minimal.

## Validation

- 9 library tests, 48 ordinary integration tests and 36 data/compiler/parser
  tests pass. A new ownership test proves a frame can be recycled while a saved
  binding retains its original value.
- The existing upstream test function still fails 19 of its 550 cases; two
  existing data fixture tests still fail on Windows CRLF. These match the
  previously recorded baseline failures in `vm-results.md` and local baseline logs.
- A separate differential run of all 550 upstream cases found no changes in
  trimmed stdout or exit status between this task's original and final release
  binaries, with no timeouts (`target/runtime-differential.json`). This is a
  regression check, not a claim that all upstream cases pass.
- Final VM integration checks cover recursion without inlining, captured
  environments, folds, nested recovery and paths. Release CLI and allocation
  probe builds succeed.

See [measurement commands and metric definitions](README.md#runtime-allocation-measurements).
