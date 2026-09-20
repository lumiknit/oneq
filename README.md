# oneq

`1q` (one-q) is an extended CLI and WASM tool compatible with [jq](https://jqlang.org/).

## Quick Start

- Web Playground: https://lumiknit.github.io/apps/1q/
- Install the CLI:

```sh
cargo install --git https://github.com/lumiknit/oneq
```

## Features

- A drop-in replacement for `jq` using `1q`
  - Features that work in the `jq` CLI, including built-ins and CLI options, are kept as compatible as possible.
  - Supports options such as `--stream`, `--stream-errors`, and `--seq`.
  - The implementation is currently based on `jq 1.8.2`; future changes in `jq` will be followed preferentially.
- Extensions
  - `--from`, `--to`: use JSON5, YAML, TOML, Python literals, environment files, CSV, XML, and more in addition to `json`.
  - `jq` itself can be selected for `--from` / `--to`, or `--fmt` can be used to edit and format jq scripts as ASTs.
  - `--doc` provides detailed built-in documentation.
  - `--repl` starts REPL mode.

### Differences with Original jq

- Uses Rust's standard `regex` engine instead of Oniguruma.
- Due to buffering, error locations may be slightly inaccurate while processing JSON and other data.

## Usage

### CLI

- Run `1q --help` for detailed usage information.
- Since `1q` keeps `jq` usage compatible wherever possible, the [jq manual](https://jqlang.org/manual/) is also useful.

### External Formats

In addition to JSON, `1q` can read and write several other formats. Specify the input and output formats with `--from` (`-F`) and `--to` (`-T`).

| Name | Description | Parser? | Serializer? (Supports compact?) | Document separator |
|-|-|-|-|
| json | (Default) JSON (RFC 8259) | Y | Y (Y) | *any (2)* |
| cbor | CBOR (RFC 8949) | Y | Y (binary; output options ignored) | none |
| json5 | JSON5 | Y | alias of `json` | *any (2)* |
| j | Loose JSON (allows most JSON-like structures) | Y | alias of `json` | *any (2)* |
| pylit | Python literal (`None`, `True`, `False`, etc.) | Y | Y | *any (2)* |
| yaml | YAML 1.2 | Y | Y (Y) | `\n---\n` |
| toml | TOML 1.0 | Y | Y (N) | `\n+++\n` |
| csv | CSV (RFC 4180) | Y | Y (N) | `\n\n` |
| csvh | CSV with header | Y | Y (N) | `\n\n` |
| tsv | TSV | Y | Y (N) | `\n\n` |
| tsvh | TSV with header | Y | Y (N) | `\n\n` |
| env | `KEY=VALUE` pairs (dotenv and exported environment syntax allowed); output is shell-compatible | Y | Y (N) | none |
| exportenv | `export KEY=VALUE` | N | Y (N) | none |
| xml | XML | Y | **P(1)** (Y) | *any (2)* |
| jq | jq syntax tree | Y | **P(1)** (Y) | none |
| raw | Same as `-R` | Y | Y | none |
| rawslurp | Same as `-R --slurp` | Y | Y | none |

**(1)** `Serializer: P`: serialization may fail if the filter result does not match the target format.

**(2)** `any`: formats such as JSON and XML do not need an explicit separator to determine the end of a document. The remaining input is treated as the end of the document, and output may add `\n`.

Format names are case-insensitive, and non-ASCII-alphanumeric characters are ignored. For example, `json`, `Json`, and `J_S_O_N` are all accepted.

Important format-related notes:

- Except for JSON, all other formats discard data that is not represented by JSON values (`null`, boolean, number, string, array, and object).
  - Unsupported values such as comments and datetimes are omitted.
  - Section order in TOML and anchors in YAML are also omitted.
- Formats other than JSON use format-specific document separators.
- Formats such as `xml` and `jq` do not map one-to-one to JSON, so serialization may fail if a filter result does not match the format schema.
- TOML has no `null`; null fields are treated as absent and omitted. This can change array lengths and similar structures.
- Most formats attempt streaming parsing with `--stream`, but some formats (for example, CSV with headers and jq) may read part or all of the input in advance.
- Options such as `--raw-input`, `--raw-output`, and `--join-output` override the formats specified by `--from`/`--to`. They run without an error, so take care when combining them.

### JSON-like Formats

JSON, JSON5, Python literals, and loose JSON are all parsed in a JSON-like way. Their differences mainly concern trailing commas and keywords such as `null`, `None`, `True`, and `false`.

`1q` accepts some inputs more leniently than `jq`:

- `Infinity`, `NaN`, and similar values are accepted even in strict JSON mode.
  - When producing strict JSON, they are converted like `jq`: `NaN` becomes `null`, while `Infinity` becomes `f64::max`, and so on.
- Keyword matching is case-insensitive.
- Duplicate fields are allowed; later occurrences overwrite earlier ones.

### Output Format Style

`1q` supports jq's `--compact-output` (`-c`). It also adds `--inline-output`, which tries to keep output on one line like compact output while remaining more readable, for example by adding spaces after commas.

These options are supported in most formats, but formats that are difficult to compress into one line, such as CSV, may produce the same output for pretty-print and compact modes.

Color options `--color-output` (`-C`) and `--monochrome-output` (`-M`) are also available. Color output is enabled by default when output goes to a TTY.

### Stream Option

With `--stream`, values are supplied to the filter as path/value pairs instead of complete JSON values. Each value is passed to the filter as soon as it is encountered.

Most formats, including JSON and YAML, do not visit a path more than once. TOML can revisit sections. It still follows the stream rules, but keep these edge cases in mind.

For example, `1q --stream -F toml -c .` passes the following values to the filter for each input line:

```toml
[a.b]
t = 42
#=> [["a","b","t"],42]

[c]
#=> [["a","b","t"]]
#=> [["a","b"]]

z = 5
#=> [["c","z"],5]

# EOF
#=> [["c","z"]]
#=> [["c"]]
```

Internally, `1q` processes parser output in this stream form by default and combines it into JSON values using these rules:

- For `[PATH, value]`, enter `PATH` and insert the value. Missing intermediate objects or arrays are created.
- `value` may be any value. (jq only allows `null`, booleans, numbers, strings, and empty objects/arrays here.)
- For `[PATH]`, do nothing if `PATH` has length two or more.
- If its length is zero or one, treat it as the end of a document and emit one JSON value, regardless of the path.

### Formatting jq Scripts

`1q` can use jq scripts as both input and output. Since compact, one-line, pretty-print, and color output are supported, it can also be used as a jq script formatter.

`--fmt` is an alias for `-F jq -T jq .` (both input and output are jq, with the filter set to pass-through).

```sh
# Format 'my_script.jq' and pretty-print it with color to stdout
cat 'my_script.jq' | 1q -F jq -T jq .

# Shorthand: --fmt. Equivalent to -F jq -T jq .
cat 'my_script.jq' | 1q --fmt
```

### REPL

Run `1q --repl` to start REPL mode. Running `1q` with no arguments on an interactive terminal also starts the REPL. A normal `1q <FILTER>` applies one fixed filter to successive stdin values. In the REPL, you can add a new filter for the current input and apply another filter to its output.

Commands beginning with `:` are available. To exit, send SIGINT (`^C`) twice. See Rustyline's behavior for details.

- `:help` or `:h`: Show help.
- `:doc` or `:d`: Show built-in function references.
- `:grammar` or `:g`: Show grammar references.
- `:exit` or `:q`: Exit the REPL.
- `:load <PATH>` or `:l`/`:file`/`:f`: Set input from a file. The format is selected from the file extension when supported.
- `:to <FORMAT>`: Select the output serializer, for example `:to yaml`.
- `:dump [PATH]`: Print the accumulated jq script, or write it to a file.

The REPL differs from normal `1q` filters in these ways:

- As with other REPLs, state can accumulate between evaluations.
- Definitions such as `as $t` and `def` are available in later filters through the global scope. **However, later evaluations use the most recently assigned `as` or `def` value, so the result can differ from simply connecting filters with a pipe.**

```jq
1,2,3 | . as $a | . | $a + .

# When `. | $a + .` runs for each 1, 2, 3
#=> 2
#=> 4
#=> 6
```

```jq-repl
1q> 1, 2, 3 | . as $a | .

# Since the initial input is null, this prints null, and $a is 3
#=> 1
#=> 2
#=> 3

1q> $a + .
#=> 4
#=> 5
#=> 6
```

---

## Project

### Status

| Level | Name | Description |
|-|-|-|
| 0 | Planned | Planned, not implemented |
| 1 | Experimental | In development and unstable; not recommended for use |
| 2 | Alpha | Implementation is stable but under testing; not recommended for use |
| 3 | Beta | Stable implementation under testing; works for most data and inputs but is not fully verified |
| 4 | Stable | Functionality is guaranteed to work |

| Feature | Level | Details |
|-|-|-|
| Data: JSON | 3 Beta | Parsing & serialization |
| Data: jq | 1 Experimental | Parsing & serialization |
| Data: Others | 2 Alpha | Parsing & serialization |
| Compiler: Lowering | 2 Alpha | . |
| Optimizer: IR-level | 0 Planned | . |
| Optimizer: CalcBlock | 0 Planned | . |
| VM: Naive Filter | 3 Beta | Some obvious fields such as `.`, and simple built-in calls |
| VM: Full VM | 2 Alpha | . |
| VM: REPL VM | 1 Experimental | . |
| jq CLI option compatibility | 3 Beta | . |
| Web UI | 2 Alpha | . |

### Tests

- Original jq test (https://github.com/jqlang/jq/blob/master/tests/jq.test)
  - 531 / 550 pass

### Benchmark

Benchmarks are maintained in `/benchmarks`. Run `go run /benchmarks/compare.go` to compare implementations.

At present, `1q` is approximately twice as slow as `jq`, `jaq`, and `gojq`.
