"""Sequential before/after CLI benchmark; includes startup, compilation and I/O."""
import argparse
import hashlib
import json
import pathlib
import platform
import statistics
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", required=True)
    parser.add_argument("--after", required=True)
    parser.add_argument("--set", action="append", default=[])
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--timeout", type=float, default=10)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("--runs must be positive")
    paths = {name: str(pathlib.Path(getattr(args, name)).resolve()) for name in ("before", "after")}
    report = {"executables": paths, "platform": platform.platform(),
              "sha256": {name: hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()
                         for name, path in paths.items()},
              "runs": args.runs, "cases": []}
    failed = False
    for directory in args.set or ["benchmarks/set", "benchmarks/vm"]:
        for query_file in sorted(pathlib.Path(directory).glob("*.jq")):
            query = query_file.read_text(encoding="utf-8")
            data = query_file.with_suffix(".jsonl").read_bytes()
            samples = {name: [] for name in paths}
            expected = None
            matches = True
            for run in range(args.runs):
                # Alternate ordering and never time competing processes together.
                order = list(paths) if run % 2 == 0 else list(reversed(paths))
                for name in order:
                    start = time.perf_counter()
                    try:
                        out = subprocess.run([paths[name], "-c", query], input=data,
                                             capture_output=True, timeout=args.timeout)
                        elapsed = time.perf_counter() - start
                        signature = (out.returncode, out.stdout, out.stderr)
                        if expected is None:
                            expected = signature
                        matches &= signature == expected and out.returncode == 0
                        samples[name].append({"seconds": elapsed, "exit_code": out.returncode,
                                              "stdout_sha256": hashlib.sha256(out.stdout).hexdigest(),
                                              "stderr": out.stderr.decode(errors="replace")})
                    except subprocess.TimeoutExpired:
                        matches = False
                        samples[name].append({"timeout": args.timeout})
            medians = {name: statistics.median([s["seconds"] for s in values])
                       for name, values in samples.items() if all("seconds" in s for s in values)}
            row = {"case": str(query_file), "matches": matches, "samples": samples,
                   "median_seconds": medians}
            if len(medians) == 2:
                row["speedup"] = medians["before"] / medians["after"]
                print(f"{query_file.stem:20} {medians['before']:8.4f}s -> {medians['after']:8.4f}s"
                      f"  {row['speedup']:6.2f}x  {'OK' if matches else 'FAIL'}", flush=True)
            else:
                print(f"{query_file.stem:20} timeout", flush=True)
            report["cases"].append(row)
            failed |= not matches
    pathlib.Path(args.output).write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    raise SystemExit(1 if failed else 0)


if __name__ == "__main__":
    main()
