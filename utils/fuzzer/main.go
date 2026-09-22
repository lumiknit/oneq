// Command fuzzer generates small, valid jq programs and JSON values, then
// compares jq and 1q on the same pair. It is intentionally self-contained so
// it can be run without changing the Rust crate or adding Go dependencies:
//
//   go run ./utils/fuzzer/main.go -oneq ./target/release/1q
//
// A mismatch is appended to fuzz.err.txt by default. The generated programs
// are not intended to be pretty; keeping them small makes a counterexample
// easier to reduce manually.
package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"math"
	"math/rand"
	"os"
	"os/exec"
	"runtime"
	"strconv"
	"strings"
	"sync"
	"time"
)

type result struct {
	exit    int
	stdout []byte
	stderr bool
	err    string
}

type sample struct {
	filter string
	input  []byte
}

func main() {
	iterations := flag.Int("iterations", 100000, "number of generated cases (0 means run forever)")
	seed := flag.Int64("seed", time.Now().UnixNano(), "random seed")
	jqPath := flag.String("jq", "jq", "jq executable")
	oneqPath := flag.String("oneq", "target/release/1q", "1q executable")
	timeout := flag.Duration("timeout", 2*time.Second, "maximum time for either tool per case")
	interval := flag.Duration("interval", 5*time.Second, "progress print interval")
	errPath := flag.String("errors", "fuzz.err.txt", "counterexample log")
	flag.Parse()

	if *iterations < 0 {
		fatal("-iterations must be non-negative")
	}
	*jqPath = executable(*jqPath)
	*oneqPath = executable(*oneqPath)
	log, err := os.OpenFile(*errPath, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0644)
	if err != nil {
		fatal("open error log: %v", err)
	}
	defer log.Close()

	rng := rand.New(rand.NewSource(*seed))
	started := time.Now()
	lastReport := started
	passed, failed := 0, 0
	failures := 0
	fmt.Printf("fuzzer seed=%d jq=%s 1q=%s\n", *seed, *jqPath, *oneqPath)

	for i := 0; *iterations == 0 || i < *iterations; i++ {
		caseData := generate(rng)
		jqResult, oneqResult := compare(caseData, *jqPath, *oneqPath, *timeout)
		if same(jqResult, oneqResult) {
			passed++
		} else {
			failed++
			failures++
			if err := writeFailure(log, i+1, *seed, caseData, jqResult, oneqResult); err != nil {
				fatal("write counterexample: %v", err)
			}
		}

		now := time.Now()
		if now.Sub(lastReport) >= *interval {
			fmt.Printf("%s cases=%d passed=%d failed=%d rate=%.1f/s\n", now.Format(time.RFC3339), passed+failed, passed, failed, float64(passed+failed)/now.Sub(started).Seconds())
			lastReport = now
		}
	}
	fmt.Printf("done cases=%d passed=%d failed=%d counterexamples=%d\n", passed+failed, passed, failed, failures)
}

func executable(path string) string {
	if strings.ContainsRune(path, os.PathSeparator) {
		if _, err := os.Stat(path); err != nil {
			fatal("executable %q: %v", path, err)
		}
		return path
	}
	found, err := exec.LookPath(path)
	if err != nil {
		fatal("executable %q: %v", path, err)
	}
	return found
}

func compare(s sample, jqPath, oneqPath string, timeout time.Duration) (result, result) {
	var jq, oneq result
	var wg sync.WaitGroup
	wg.Add(2)
	go func() { defer wg.Done(); jq = run(jqPath, s.filter, s.input, timeout) }()
	go func() { defer wg.Done(); oneq = run(oneqPath, s.filter, s.input, timeout) }()
	wg.Wait()
	return jq, oneq
}

func run(path, filter string, input []byte, timeout time.Duration) result {
	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()
	cmd := exec.CommandContext(ctx, path, "-c", filter)
	cmd.Stdin = bytes.NewReader(input)
	var stdout, stderr bytes.Buffer
	cmd.Stdout, cmd.Stderr = &stdout, &stderr
	err := cmd.Run()
	exit := 0
	if cmd.ProcessState != nil {
		exit = cmd.ProcessState.ExitCode()
	} else {
		exit = -1
	}
	if ctx.Err() != nil {
		exit = -1
	}
	return result{exit: exit, stdout: stdout.Bytes(), stderr: stderr.Len() != 0, err: commandError(err)}
}

func commandError(err error) string {
	if err == nil || errors.Is(err, os.ErrProcessDone) {
		return ""
	}
	return err.Error()
}

func same(a, b result) bool {
	return a.exit == b.exit && bytes.Equal(a.stdout, b.stdout) && a.stderr == b.stderr
}

func writeFailure(log *os.File, n int, seed int64, s sample, jq, oneq result) error {
	var b strings.Builder
	fmt.Fprintf(&b, "\n=== mismatch case=%d seed=%d platform=%s/%s ===\n", n, seed, runtime.GOOS, runtime.GOARCH)
	fmt.Fprintf(&b, "filter: %s\ninput: %s\n", s.filter, s.input)
	writeResult(&b, "jq", jq)
	writeResult(&b, "1q", oneq)
	_, err := log.WriteString(b.String())
	return err
}

func writeResult(b *strings.Builder, name string, r result) {
	fmt.Fprintf(b, "%s: exit=%d stderr=%t err=%q stdout=%q\n", name, r.exit, r.stderr, r.err, r.stdout)
}

func fatal(format string, args ...any) {
	fmt.Fprintf(os.Stderr, "fuzzer: "+format+"\n", args...)
	os.Exit(2)
}

// generate uses a deliberately small grammar. Every production below is jq
// syntax; runtime type errors are useful because try/catch and error paths are
// part of what is being compared.
func generate(r *rand.Rand) sample {
	depth := 2 + r.Intn(4)
	filter := expr(r, depth)
	input := jsonValue(r, 0)
	data, err := json.Marshal(input)
	if err != nil {
		panic(err)
	}
	return sample{filter: filter, input: append(data, '\n')}
}

func expr(r *rand.Rand, d int) string {
	if d <= 0 {
		return leaf(r)
	}
	child := func() string { return expr(r, d-1) }
	switch r.Intn(36) {
	case 0:
		return ". | " + child()
	case 1:
		return child() + ", " + child()
	case 2:
		return "[" + child() + "]"
	case 3:
		return "{" + key(r) + ": " + child() + ", x: " + child() + "}"
	case 4:
		return "try (" + child() + ") catch (" + child() + ")"
	case 5:
		// try without catch is equivalent to catching with empty.
		return "try (" + child() + ")"
	case 6:
		return "if (" + child() + " | type == \"boolean\") then (" + child() + ") else (" + child() + ") end"
	case 7:
		return "if (" + child() + " | type == \"null\") then (" + child() + ") elif (" + child() + " | type == \"array\") then (" + child() + ") else (" + child() + ") end"
	case 8:
		return "range(0; " + strconv.Itoa(1+r.Intn(4)) + ") | (" + child() + ")"
	case 9:
		return "reduce range(0; " + strconv.Itoa(1+r.Intn(4)) + ") as $i (0; . + $i)"
	case 10:
		return "foreach range(0; " + strconv.Itoa(1+r.Intn(4)) + ") as $i (0; . + $i; .)"
	case 11:
		return ". as $x | [($x), (" + child() + ")]"
	case 12:
		return ". as {$a, $b} | [$a, $b, (" + child() + ")]"
	case 13:
		return "label $out | (" + child() + ") , break $out"
	case 14:
		return "path(" + pathExpr(r) + ")"
	case 15:
		return "(" + child() + ") // (" + child() + ")"
	case 16:
		return "empty, (" + child() + ")"
	case 17:
		return "(" + child() + ") | ., ."
	case 18:
		return "(" + child() + ") | length"
	case 19:
		return "(" + child() + ") | " + builtin(r)
	case 20:
		return "(" + updateTarget(r) + ") |= (" + child() + ")"
	case 21:
		return "(" + updateTarget(r) + ") = (" + child() + ")"
	case 22:
		return "(" + updateTarget(r) + ") += " + strconv.Itoa(r.Intn(3)-1)
	case 23:
		return "(" + updateTarget(r) + ") //= (" + child() + ")"
	case 24:
		return "setpath([\"a\", 0]; (" + child() + "))"
	case 25:
		return "del(" + updateTarget(r) + ")"
	case 26:
		return "map((" + child() + "))"
	case 27:
		return "map_values((" + child() + "))"
	case 28:
		return "limit(2; (" + child() + "))"
	case 29:
		return "first((" + child() + "))"
	case 30:
		return "last((" + child() + "))"
	case 31:
		return "nth(0; (" + child() + "))"
	case 32:
		return "recurse(.[]?) | (" + child() + ")"
	case 33:
		return "(" + child() + ") | (select(. != null) // null)"
	case 34:
		return "[" + child() + "] | {items: ., count: length}"
	default:
		return "[" + child() + "] | add"
	}
}

func updateTarget(r *rand.Rand) string {
	choices := []string{".a", ".b", ".[0]", ".[\"a\"]", ".a[0]"}
	return choices[r.Intn(len(choices))]
}

func leaf(r *rand.Rand) string {
	choices := []string{".", ".[]?", ".a", ".a[]?", "empty", "null", "true", "false", "1", "-2", "\"a\"", "\"a,b\"", "[1, 2, 3]", "{a: 1, b: \"x\"}"}
	return choices[r.Intn(len(choices))]
}

func builtin(r *rand.Rand) string {
	choices := []string{
		"abs", "floor", "ceil", "round", "sqrt", "fabs", "tostring", "tonumber",
		"type", "length", "keys", "values", "sort", "unique", "flatten",
		"ascii_downcase", "ascii_upcase", "explode", "split(\",\")", "join(\"-\")",
		"startswith(\"a\")", "endswith(\"z\")", "ltrimstr(\"a\")", "rtrimstr(\"z\")",
		"to_entries", "from_entries", "min", "max", "todateiso8601",
	}
	return choices[r.Intn(len(choices))]
}

func pathExpr(r *rand.Rand) string {
	choices := []string{".", ".a", ".[]", ".a[]", "..", ".[0]", ".[\"a\"]"}
	return choices[r.Intn(len(choices))]
}

func key(r *rand.Rand) string {
	keys := []string{"a", "b", "value", "kind"}
	return keys[r.Intn(len(keys))]
}

func jsonValue(r *rand.Rand, depth int) any {
	if depth >= 4 {
		return jsonLeaf(r)
	}
	// These cases are intentionally explicit: empty containers, null slots,
	// unusual keys, repeated shapes and long arrays expose path/update and
	// number/string representation bugs much more reliably than happy-path JSON.
	switch r.Intn(18) {
	case 0:
		return nil
	case 1:
		return []any{}
	case 2:
		return []any{[]any{}}
	case 3:
		return []any{nil}
	case 4:
		return map[string]any{"a": nil}
	case 5:
		return map[string]any{"a": []any{}}
	case 6:
		return map[string]any{"a": map[string]any{"a": map[string]any{"a": 1}}}
	case 7:
		return map[string]any{"": 1}
	case 8:
		return map[string]any{"0": 1}
	case 9:
		return map[string]any{"a": jsonValue(r, depth+1), "b": jsonValue(r, depth+1)}
	case 10:
		return map[string]any{"value": r.Intn(7) - 3, "kind": "x"}
	case 11:
		item := map[string]any{"a": nil, "value": jsonLeaf(r)}
		return []any{item, item, item, item, item, item, item, item}
	case 12:
		return []any{r.Intn(5), r.Intn(5), "a", true}
	case 13:
		items := make([]any, 16)
		for i := range items {
			items[i] = jsonValue(r, depth+1)
		}
		return items
	case 14:
		return jsonLeaf(r)
	case 15:
		return []any{jsonValue(r, depth+1), jsonValue(r, depth+1)}
	case 16:
		return map[string]any{"a": jsonValue(r, depth+1), "b": jsonValue(r, depth+1)}
	default:
		return map[string]any{}
	}
}

func jsonLeaf(r *rand.Rand) any {
	values := []any{
		nil, true, false, 0, 1, -1, math.Copysign(0, -1),
		json.Number("9007199254740993"),
		json.Number("9223372036854775807"),
		json.Number("-9223372036854775808"),
		math.SmallestNonzeroFloat64, math.MaxFloat64,
		"", "a", "a,b", "こんにちは", "😀\n\t\\\"", "\\u0000\\n\\t",
		"2020-01-01T00:00:00Z",
	}
	return values[r.Intn(len(values))]
}
