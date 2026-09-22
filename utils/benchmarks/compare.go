// compare.go runs the jq-compatible filters in benchmarks/set.
//
// A process is started for each tool before the clock starts. The clock then
// covers writing the JSONL input (including EOF) and waiting for the process
// to exit. Remaining process startup and filter compilation are included;
// starting a process does not wait for its filter to finish compiling.
package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"sort"
	"strings"
	"sync"
	"time"
)

type result struct {
	Tool     string  `json:"tool"`
	Seconds  float64 `json:"seconds"`
	ExitCode int     `json:"exit_code"`
	Output   []byte  `json:"output"`
	Stderr   string  `json:"stderr,omitempty"`
	Error    string  `json:"error,omitempty"`
}

type reportCase struct {
	Case    string   `json:"case"`
	Filter  string   `json:"filter"`
	Results []result `json:"results"`
}

type report struct {
	Platform string       `json:"platform"`
	Set      string       `json:"set"`
	Runs     int          `json:"runs"`
	Cases    []reportCase `json:"cases"`
}

var toolOrder = []string{"jq", "jaq", "gojq", "1q"}

func main() {
	setDir := flag.String("set", "utils/benchmarks/set", "directory containing CASE.jq and CASE.jsonl")
	jqPath := flag.String("jq", "jq", "jq executable")
	jaqPath := flag.String("jaq", "jaq", "jaq executable")
	gojqPath := flag.String("gojq", "gojq", "gojq executable")
	oneqPath := flag.String("oneq", "target/release/1q", "1q executable")
	runs := flag.Int("runs", 1, "number of input repetitions in each process")
	timeout := flag.Duration("timeout", 2*time.Minute, "maximum time for one process")
	outputPath := flag.String("output", "", "write the detailed report as JSON")
	flag.Parse()

	if *runs < 1 {
		fatal("-runs must be positive")
	}

	paths := map[string]string{"jq": *jqPath, "jaq": *jaqPath, "gojq": *gojqPath, "1q": *oneqPath}
	for _, name := range toolOrder {
		paths[name] = findExecutable(paths[name])
	}

	entries, err := os.ReadDir(*setDir)
	if err != nil {
		fatal("read set directory: %v", err)
	}
	var names []string
	for _, entry := range entries {
		if before, ok := strings.CutSuffix(entry.Name(), ".jq"); ok {
			name := before
			if _, err := os.Stat(filepath.Join(*setDir, name+".jsonl")); err == nil {
				names = append(names, name)
			}
		}
	}
	sort.Strings(names)
	if len(names) == 0 {
		fatal("no CASE.jq/CASE.jsonl pairs found in %s", *setDir)
	}

	reportData := report{Platform: runtime.GOOS + "/" + runtime.GOARCH, Set: *setDir, Runs: *runs}
	fmt.Printf("%-24s %-8s %12s %10s %s\n", "case", "tool", "time", "vs jq", "status")
	for _, name := range names {
		filter, err := os.ReadFile(filepath.Join(*setDir, name+".jq"))
		if err != nil {
			fatal("read %s.jq: %v", name, err)
		}
		input, err := os.ReadFile(filepath.Join(*setDir, name+".jsonl"))
		if err != nil {
			fatal("read %s.jsonl: %v", name, err)
		}

		caseReport := reportCase{Case: name, Filter: string(filter)}
		{
			results := runCase(string(filter), input, *runs, paths, *timeout)
			caseReport.Results = append(caseReport.Results, results...)
			printResults(name, results)
			if mismatches(results) {
				printMismatch(name, results)
			}
		}
		fmt.Println("--------------------")
		reportData.Cases = append(reportData.Cases, caseReport)
	}

	if *outputPath != "" {
		data, err := json.MarshalIndent(reportData, "", "  ")
		if err != nil {
			fatal("encode report: %v", err)
		}
		if err := os.WriteFile(*outputPath, append(data, '\n'), 0644); err != nil {
			fatal("write report: %v", err)
		}
	}
}

func findExecutable(path string) string {
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

func runCase(filter string, input []byte, runs int, paths map[string]string, timeout time.Duration) []result {
	input = bytes.Repeat(input, runs)
	type running struct {
		name     string
		cmd      *exec.Cmd
		stdin    io.WriteCloser
		out, err bytes.Buffer
	}
	runningTools := make([]*running, 0, len(toolOrder))
	for _, name := range toolOrder {
		ctx, cancel := context.WithTimeout(context.Background(), timeout)
		cmd := exec.CommandContext(ctx, paths[name], "-c", filter)
		stdin, err := cmd.StdinPipe()
		if err != nil {
			fatal("create stdin for %s: %v", name, err)
		}
		r := &running{name: name, cmd: cmd, stdin: stdin}
		cmd.Stdout, cmd.Stderr = &r.out, &r.err
		if err := cmd.Start(); err != nil {
			cancel()
			fatal("start %s: %v", name, err)
		}
		// Keep the context alive through Wait, but release it after this run.
		runningTools = append(runningTools, r)
		_ = cancel
	}

	started := time.Now()
	var wg sync.WaitGroup
	for _, r := range runningTools {
		wg.Add(1)
		go func(r *running) {
			defer wg.Done()
			_, writeErr := r.stdin.Write(input)
			closeErr := r.stdin.Close() // EOF is part of the measured operation.
			if writeErr != nil {
				r.err.WriteString(writeErr.Error())
			}
			if closeErr != nil {
				r.err.WriteString(closeErr.Error())
			}
		}(r)
	}
	wg.Wait()

	results := make([]result, len(runningTools))
	var waits sync.WaitGroup
	for i, r := range runningTools {
		waits.Add(1)
		go func(i int, r *running) {
			defer waits.Done()
			err := r.cmd.Wait()
			code := -1
			if r.cmd.ProcessState != nil {
				code = r.cmd.ProcessState.ExitCode()
			}
			results[i] = result{
				Tool:     r.name,
				Seconds:  time.Since(started).Seconds() / float64(runs),
				ExitCode: code,
				Output:   bytes.ReplaceAll(r.out.Bytes(), []byte("\r\n"), []byte("\n")),
				Stderr:   r.err.String(),
				Error:    commandError(err),
			}
		}(i, r)
	}
	waits.Wait()
	return results
}

func commandError(err error) string {
	if err == nil || errors.Is(err, os.ErrProcessDone) {
		return ""
	}
	return err.Error()
}

func printResults(name string, results []result) {
	jqTime := results[0].Seconds
	for _, r := range results {
		ratio := jqTime / r.Seconds
		status := fmt.Sprintf("exit=%d", r.ExitCode)
		if r.Error != "" {
			status += " " + r.Error
		}
		fmt.Printf("%-24s %-8s %9.3f ms %10.2fx faster %s\n", name, r.Tool, r.Seconds*1000, ratio, status)
	}
}

func mismatches(results []result) bool {
	if len(results) == 0 {
		return false
	}
	for _, r := range results[1:] {
		if r.ExitCode != results[0].ExitCode || !bytes.Equal(r.Output, results[0].Output) {
			return true
		}
	}
	return false
}

func printMismatch(name string, results []result) {
	fmt.Printf("  MISMATCH %s (jq is the reference) -", name)
	for _, r := range results {
		fmt.Printf(" %s(Exit %d, %d B", r.Tool, r.ExitCode, len(r.Output))
		if r.Stderr != "" {
			fmt.Printf(", err=%q", r.Stderr)
		}
		fmt.Print(")")
	}
	fmt.Println()
}

func fatal(format string, args ...any) {
	fmt.Fprintf(os.Stderr, "compare: "+format+"\n", args...)
	os.Exit(1)
}
