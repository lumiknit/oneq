# CLI benchmarks

Build the release binary, then compare jq, jaq, gojq and 1q:

```sh
cargo build --release
go run benchmarks/compare.go -jq ./.manual/jq.exe -oneq ./target/release/1q.exe -runs 1 -timeout 10s
go run benchmarks/compare.go -jq ./.manual/jq.exe -oneq ./target/release/1q.exe -set benchmarks/vm -runs 1 -timeout 10s
```

Override `-jaq` and `-gojq` if they are not on PATH. On Unix, use the appropriate
executable paths without `.exe`. `-output report.json` saves samples and outputs.
The Go driver runs tools concurrently and checks output against jq. Its `-runs`
option repeats input in one process; it does **not** take independent samples.
Timing starts after process creation, but includes any remaining startup and
compilation, input processing, execution, output and process exit.

For changes to the VM, preserve the old release binary before rebuilding, then
measure before/after independently:

```sh
# Before changing the code (PowerShell):
Copy-Item target/release/1q.exe target/release/1q-before.exe
# After rebuilding:
python benchmarks/vm_compare.py --before target/release/1q-before.exe --after target/release/1q.exe --runs 3 --output target/vm-final.json
```

This standard-library-only Python driver runs one process at a time, alternates
before/after order, reports medians, and requires identical stdout, stderr and
successful exit codes on every run. It includes startup and compilation, so
short cases have a significant fixed cost. `--set` can be repeated to select
case directories; defaults are `benchmarks/set` and `benchmarks/vm`.

The VM cases cover full array iteration, abandoning iteration after the first
item, nested recovery handlers and path tracking. The ordinary suite also covers
recursion, arithmetic and native streams. Run on an idle machine without builds
or tests competing with the measured processes. The release profile uses
`opt-level = 3`, fat LTO and one codegen unit.

## Runtime allocation measurements

```sh
cargo build --release --example runtime_alloc
./target/release/examples/runtime_alloc benchmarks/set/add.jq
./target/release/examples/runtime_alloc benchmarks/set/math.jq
./target/release/examples/runtime_alloc benchmarks/set/fibo.jq
```

Use `.exe` on Windows. The probe reads the adjacent `.jsonl` file and compiles
the query before resetting the counters. It executes through `Session`, consumes
and drops each output without rendering it, and reports:

- Successful allocation and reallocation requests during execution.
- Cumulative requested bytes (a reallocation counts its entire new size).
- Live requested heap bytes before execution, the absolute peak, and the peak
  increase over that baseline.

These are requested Rust heap sizes, excluding allocator metadata, stack memory,
and OS/library allocations outside Rust's global allocator. They are neither
process RSS nor bytes actually read/written by the CPU. The probe's atomic
counters perturb execution time; use the ordinary release CLI for timing.
The baseline is subtracted from the absolute peak; this is a net live-heap
increase, not allocation-lifetime attribution to individual runtime objects.

Windows process working-set peaks can be collected with
[GetProcessMemoryInfo](https://learn.microsoft.com/en-us/windows/win32/procthread/process-working-set).
Loads/stores, cache misses and DRAM bandwidth require hardware-counter profiling,
for example [VTune Memory Access](https://www.intel.com/content/www/us/en/docs/vtune-profiler/user-guide/2024-0/memory-access-analysis.html)
on supported hardware. Allocation traffic alone cannot establish a DRAM bandwidth bottleneck.
