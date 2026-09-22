// Command fuzzer generates small, valid jq programs and JSON values, then
// compares jq and 1q on the same pair. It is intentionally self-contained so
// it can be run without changing the Rust crate or adding Go dependencies:
//
//	go run ./utils/fuzzer/main.go -oneq ./target/release/1q
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
	"sync/atomic"
	"time"
)

type result struct {
	exit   int
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
	workers := flag.Int("workers", defaultWorkers(), "number of cases to run in parallel")
	flag.Parse()

	if *iterations < 0 {
		fatal("-iterations must be non-negative")
	}
	if *workers < 1 {
		fatal("-workers must be positive")
	}
	*jqPath = executable(*jqPath)
	*oneqPath = executable(*oneqPath)
	log, err := os.OpenFile(*errPath, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0644)
	if err != nil {
		fatal("open error log: %v", err)
	}
	defer log.Close()

	started := time.Now()
	var issued, passed, failed int64
	var logMu sync.Mutex
	fmt.Printf("fuzzer seed=%d workers=%d jq=%s 1q=%s\n", *seed, *workers, *jqPath, *oneqPath)

	done := make(chan struct{})
	var wg sync.WaitGroup
	for w := 0; w < *workers; w++ {
		wg.Go(func() {
			for {
				n := atomic.AddInt64(&issued, 1)
				if *iterations != 0 && n > int64(*iterations) {
					return
				}
				// Deriving the rng from the case number keeps a case
				// reproducible no matter how many workers ran it.
				rng := rand.New(rand.NewSource(int64(uint64(*seed) + uint64(n)*0x9e3779b97f4a7c15)))
				caseData := generate(rng)
				jqResult, oneqResult := compare(caseData, *jqPath, *oneqPath, *timeout)
				if same(jqResult, oneqResult) {
					atomic.AddInt64(&passed, 1)
					continue
				}
				atomic.AddInt64(&failed, 1)
				logMu.Lock()
				err := writeFailure(log, int(n), *seed, caseData, jqResult, oneqResult)
				logMu.Unlock()
				if err != nil {
					fatal("write counterexample: %v", err)
				}
			}
		})
	}

	go func() {
		ticker := time.NewTicker(*interval)
		defer ticker.Stop()
		for {
			select {
			case <-done:
				return
			case now := <-ticker.C:
				p, f := atomic.LoadInt64(&passed), atomic.LoadInt64(&failed)
				fmt.Printf("%s cases=%d passed=%d failed=%d rate=%.1f/s\n", now.Format(time.RFC3339), p+f, p, f, float64(p+f)/now.Sub(started).Seconds())
			}
		}
	}()

	wg.Wait()
	close(done)
	p, f := atomic.LoadInt64(&passed), atomic.LoadInt64(&failed)
	fmt.Printf("done cases=%d passed=%d failed=%d counterexamples=%d\n", p+f, p, f, f)
}

func defaultWorkers() int {
	if n := runtime.NumCPU(); n < 8 {
		return n
	}
	return 8
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
	// Rejecting the program at compile time (exit 3) instead of failing on it
	// at run time (exit 5) is not a difference worth reporting: both refuse
	// the same program, and constant folding decides which one happens. The
	// tool that compiled may also have printed output before failing, so the
	// comparison has to stop at the exit code here.
	if a.exit != b.exit {
		return rejected(a.exit) && rejected(b.exit)
	}
	return bytes.Equal(a.stdout, b.stdout) && a.stderr == b.stderr
}

func rejected(exit int) bool { return exit == 3 || exit == 5 }

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
	switch r.Intn(60) {
	case 0:
		return ". | " + child()
	case 1:
		return child() + ", " + child()
	case 2:
		return "[" + child() + "]"
	case 3:
		return "{" + key(r) + ": (" + child() + "), x: (" + child() + ")}"
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
	case 35:
		return "(" + child() + ") " + arithOp(r) + " (" + child() + ")"
	case 36:
		return "(" + child() + ") " + compareOp(r) + " (" + child() + ")"
	case 37:
		return "(" + child() + ") " + boolOp(r) + " (" + child() + ")"
	case 38:
		return "(" + child() + ") | not"
	case 39:
		return "-(" + child() + ")"
	case 40:
		return "\"v=\\(" + child() + ")\""
	case 41:
		return format(r) + " \"v=\\(" + child() + ")\""
	case 42:
		return "(" + child() + ") | " + format(r)
	case 43:
		return "(" + child() + ") | " + slice(r)
	case 44:
		return "(" + sliceTarget(r) + ") = (" + child() + ")"
	case 45:
		return "(" + child() + ") | " + regexCall(r)
	case 46:
		return funcDef(r, child)
	case 47:
		return "(" + updateTarget(r) + ") " + assignOp(r) + " (" + child() + ")"
	case 48:
		return reduceExpr(r, child)
	case 49:
		return foreachExpr(r, child)
	case 50:
		return patternExpr(r, child)
	case 51:
		return objectExpr(r, child)
	case 52:
		return streamExpr(r, child)
	case 53:
		return "(" + child() + ") | " + byBuiltin(r) + "((" + child() + "))"
	case 54:
		return loopExpr(r, child)
	case 55:
		return "(" + child() + ") | " + predicate(r, child)
	case 56:
		return errorExpr(r, child)
	case 57:
		return "(" + child() + ")?"
	case 58:
		return "(def f: def g: (" + child() + "); [g, g]; f)"
	default:
		return "[" + child() + "] | add"
	}
}

func updateTarget(r *rand.Rand) string {
	choices := []string{".a", ".b", ".[0]", ".[\"a\"]", ".a[0]"}
	return choices[r.Intn(len(choices))]
}

func reduceExpr(r *rand.Rand, child func() string) string {
	switch r.Intn(4) {
	case 0:
		return "reduce (" + child() + ") as $i ([]; . + [$i])"
	case 1:
		return "reduce (" + child() + ") as $i ((" + child() + "); (" + child() + "))"
	case 2:
		return "reduce (" + child() + ") as [$a, $b] ([]; . + [$a, $b])"
	default:
		return "reduce (" + child() + ") as {$a} ([]; . + [$a])"
	}
}

func foreachExpr(r *rand.Rand, child func() string) string {
	switch r.Intn(3) {
	case 0:
		return "[foreach (" + child() + ") as $i ([]; . + [$i]; length)]"
	case 1:
		return "[foreach (" + child() + ") as $i ((" + child() + "); (" + child() + "); (" + child() + "))]"
	default:
		return "[foreach (" + child() + ") as [$a] ([]; . + [$a])]"
	}
}

// `?//` needs jq 1.7 or newer; the alternatives are tried left to right and
// every variable in the union is bound (to null) in each branch.
func patternExpr(r *rand.Rand, child func() string) string {
	switch r.Intn(4) {
	case 0:
		return "(. as [$a] ?// {$a} | [$a, (" + child() + ")])"
	case 1:
		return "(. as [$a, $b] | [$a, $b])"
	case 2:
		return "(. as {a: $x, b: [$y]} | [$x, $y])"
	default:
		return "((" + child() + ") as [$a] ?// $a | [$a])"
	}
}

func objectExpr(r *rand.Rand, child func() string) string {
	switch r.Intn(5) {
	case 0:
		return "(. as $x | {$x})"
	case 1:
		return "{(" + child() + "): (" + child() + ")}"
	case 2:
		return "{\"k\\(" + child() + ")\": (" + child() + ")}"
	case 3:
		return "{a, b}"
	default:
		return "({" + key(r) + ": (" + child() + ")} + {x: 1})"
	}
}

func streamExpr(r *rand.Rand, child func() string) string {
	switch r.Intn(6) {
	case 0:
		return "[(" + child() + ") | tostream]"
	case 1:
		return "fromstream((" + child() + ") | tostream)"
	case 2:
		return "[(" + child() + ") | paths]"
	case 3:
		return "[1 | truncate_stream((" + child() + ") | tostream)]"
	case 4:
		return "(" + child() + ") | getpath([\"a\", 0])"
	default:
		return "(" + child() + ") | with_entries(.value = (" + child() + "))"
	}
}

func byBuiltin(r *rand.Rand) string {
	choices := []string{"sort_by", "group_by", "unique_by", "min_by", "max_by", "map"}
	return choices[r.Intn(len(choices))]
}

func loopExpr(r *rand.Rand, child func() string) string {
	// Every loop is bounded: `limit` caps the unbounded generators, and the
	// `until` condition shrinks its input by one element per step.
	switch r.Intn(5) {
	case 0:
		return "[limit(3; repeat((" + child() + ")))]"
	case 1:
		return "[limit(3; while(true; (" + child() + ")))]"
	case 2:
		return "([" + child() + "] | until(length == 0; .[1:]))"
	case 3:
		return "isempty((" + child() + "))"
	default:
		return "walk(if type == \"array\" then sort else (" + child() + ") end)"
	}
}

func predicate(r *rand.Rand, child func() string) string {
	switch r.Intn(7) {
	case 0:
		return "any(.[]?; (" + child() + ") != null)"
	case 1:
		return "all(.[]?; (" + child() + ") != null)"
	case 2:
		return "has(\"a\")"
	case 3:
		return "contains((" + child() + "))"
	case 4:
		return "inside((" + child() + "))"
	case 5:
		return "IN((" + child() + "))"
	default:
		return "index((" + child() + "))"
	}
}

func errorExpr(r *rand.Rand, child func() string) string {
	switch r.Intn(4) {
	case 0:
		return "try (error((" + child() + "))) catch ."
	case 1:
		return "try ((" + child() + ") | error) catch ."
	case 2:
		return "((" + child() + ") | tostring | @base64 | @base64d)"
	default:
		return "(label $a | (label $b | (" + child() + "), break $b), break $a)"
	}
}

func arithOp(r *rand.Rand) string {
	choices := []string{"+", "-", "*", "/", "%"}
	return choices[r.Intn(len(choices))]
}

func compareOp(r *rand.Rand) string {
	choices := []string{"==", "!=", "<", "<=", ">", ">="}
	return choices[r.Intn(len(choices))]
}

func boolOp(r *rand.Rand) string {
	if r.Intn(2) == 0 {
		return "and"
	}
	return "or"
}

func assignOp(r *rand.Rand) string {
	choices := []string{"|=", "=", "+=", "-=", "*=", "/=", "%=", "//="}
	return choices[r.Intn(len(choices))]
}

func format(r *rand.Rand) string {
	choices := []string{"@text", "@json", "@csv", "@tsv", "@html", "@uri", "@sh", "@base64"}
	return choices[r.Intn(len(choices))]
}

func slice(r *rand.Rand) string {
	choices := []string{".[1:3]", ".[:2]", ".[1:]", ".[-2:]", ".[0:0]", ".[2:1]"}
	return choices[r.Intn(len(choices))]
}

func sliceTarget(r *rand.Rand) string {
	choices := []string{".[1:2]", ".[:1]", ".[1:]", ".a[0:1]"}
	return choices[r.Intn(len(choices))]
}

// Patterns stay simple on purpose: jq uses Oniguruma, so exotic syntax would
// report engine differences instead of jq/1q differences.
func regexCall(r *rand.Rand) string {
	choices := []string{
		"test(\"a\")", "test(\"[ab]+\"; \"i\")", "match(\"a\")",
		"[match(\"a\"; \"g\") | .offset]", "capture(\"(?<x>a)\")",
		"[scan(\"a\")]", "split(\"a\"; null)", "split(\"[,;]\"; \"g\")",
		"sub(\"a\"; \"X\")", "gsub(\"a\"; \"X\"; \"g\")", "[splits(\"a\")]",
	}
	return choices[r.Intn(len(choices))]
}

// The def is parenthesized so it can appear anywhere a term can; otherwise its
// body would swallow the rest of the surrounding pipeline.
func funcDef(r *rand.Rand, child func() string) string {
	switch r.Intn(4) {
	case 0:
		return "(def f: (" + child() + "); f)"
	case 1:
		return "(def f(g): [g, g]; f(" + child() + "))"
	case 2:
		return "(def f($x): [$x, (" + child() + ")]; f(1))"
	default:
		// Terminates because every recursive call strips one array level.
		return "(def f: if type == \"array\" then map(f) else (" + child() + ") end; f)"
	}
}

func leaf(r *rand.Rand) string {
	choices := []string{".", ".[]?", ".a", ".a[]?", "empty", "null", "true", "false", "1", "-2", "0.5", "1e3", "1e1000", "-0", "9007199254740993", "\"a\"", "\"a,b\"", "[1, 2, 3]", "{a: 1, b: \"x\"}"}
	return choices[r.Intn(len(choices))]
}

func builtin(r *rand.Rand) string {
	choices := []string{
		"abs", "floor", "ceil", "round", "sqrt", "fabs", "tostring", "tonumber",
		"type", "length", "keys", "values", "sort", "unique", "flatten",
		"ascii_downcase", "ascii_upcase", "explode", "split(\",\")", "join(\"-\")",
		"startswith(\"a\")", "endswith(\"z\")", "ltrimstr(\"a\")", "rtrimstr(\"z\")",
		"to_entries", "from_entries", "min", "max", "todateiso8601",
		"add", "reverse", "not", "tojson", "implode", "transpose", "infinite",
		"exp", "log", "log2", "log10", "exp2", "exp10", "cbrt", "trunc",
		"nearbyint", "significand", "logb", "lgamma", "tgamma",
		"gmtime", "mktime", "todate", "fromdate", "strftime(\"%Y-%m\")",
		"toboolean", "ascii_downcase", "getpath([])", "paths",
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
