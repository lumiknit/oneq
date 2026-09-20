# WASM execution prototype

The browser build shares the CLI parser, evaluator and serializers. Its only
application entrypoint is `run(packet: Uint8Array): number`, implemented in
`src/lib_wasm.rs`. The return value is the CLI exit code. No server execution,
REPL, filesystem or external processes are involved.

## Build

Install the Rust target and the wasm-bindgen CLI version matching Cargo.lock
(currently 0.2.128), then build from the repository root:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128 --locked
sh web/build.sh
```

`WASM_BINDGEN=/path/to/wasm-bindgen sh web/build.sh` selects an existing tool.
The generated ES module, WASM and JS snippets live in `web/pkg/` (gitignored).
Deploy them together with `web/worker.js`; serve `.wasm` as `application/wasm`.
The build uses `--lib`: the native `1q` executable is not a browser entrypoint.

## Input and streaming output

Each request is UTF-8 JSON encoding of an argv array **without** `1q`, followed
by one NUL byte, followed by all stdin bytes. The array includes the filter.
Only the first NUL separates the header; NULs in stdin are preserved. Actual
NUL characters in decoded argv strings are rejected, just as OS argv cannot
contain them. stdin is normally UTF-8 text but is passed to the existing input
parser as bytes, without a whole-input UTF-8 conversion.

```js
const worker = new Worker("./web/worker.js", { type: "module" });
const encoder = new TextEncoder();
const header = encoder.encode(JSON.stringify(["-c", ".[]"]));
const stdin = encoder.encode('[1,2,3]');
const packet = new Uint8Array(header.length + 1 + stdin.length);
packet.set(header);
packet.set(stdin, header.length + 1); // the separator is already zero

const decoders = { 1: new TextDecoder(), 2: new TextDecoder() };
worker.onmessage = ({ data }) => {
  if (data.type === "output") {
    // 1 = stdout, 2 = stderr. Keep separate decoder/render state per channel.
    const text = decoders[data.channel].decode(data.bytes, { stream: true });
    appendOutput(data.channel, text); // supplied by your frontend
  } else if (data.type === "done") {
    for (const channel of [1, 2]) appendOutput(channel, decoders[channel].decode());
    showExitCode(data.code); // supplied by your frontend
  } else if (data.type === "failure") {
    // A JS exception/WASM trap is not a normal filter error. Recreate this Worker.
    showFailure(data.message);
    worker.terminate();
  }
};
worker.postMessage(packet, [packet.buffer]);
// To stop a long/infinite execution: worker.terminate(), then create a new one.
```

`wasm-output.js` is the imported output callback. It copies WASM byte slices
before transferring them to the frontend. Output is sent during `run`, not
collected until completion. Chunk boundaries are arbitrary, including inside
UTF-8/ANSI sequences. ANSI escapes from `-C` and NUL bytes from `--raw-output0`
are retained. An ANSI renderer belongs in the frontend; this prototype does
not include one. `-M` disables color; automatic color defaults to off.

Send one request at a time when attributing output to a run. The synchronous
call executes in the Worker, so the page remains responsive. `--stream` still
means jq's input streaming mode; collecting operators such as `sort`, array
constructors and `-s` may need to consume their input before producing output.
The entire stdin packet is supplied up front, not incrementally uploaded.

## Supported scope

- Existing in-memory CLI flags, including `--fmt`, `-F`/`-T`, `--arg`,
  `--argjson`, `--args`, `--jsonargs`, raw input/output, colors and `-e`.
- stdout and stderr remain separate, including `debug`, `stderr`, errors,
  help and `halt_error`. Previously emitted output survives later errors.
- File flags (`-f`, `-L`, `--rawfile`, `--slurpfile`, `-i`), input file
  arguments, `--with` and subcommands return an unsupported-operation error.
- `import` and `include` return a compile error; no virtual filesystem.
- `$ENV` and `env` are empty objects; `now` uses the JavaScript clock.
- Every call starts a new session. Normal errors return a code and allow a
  later call. Traps or forced cancellation should discard the Worker.

Malformed packets and unsupported options return 2. Normal execution uses
existing CLI status handling (including compile error 3 and input error 4).
The binding tool also exports initialization/memory helpers; `run` is the
single application operation.

## Tests

After building both targets:

```sh
cargo build --bin 1q
node web/test.mjs
node web/test-worker.mjs
```

This instantiates the actual browser-target WASM in Node, supplies its output
callback, and compares stdout bytes, stderr bytes and exit codes against the
native executable. It covers all file-free jq-oracle fixtures plus packet
validation, unsupported flags, ANSI/NUL preservation, `--fmt`, streaming order
and repeated calls. `ONEQ_BIN` can select another native executable. The
existing Rust oracle tests separately compare the native executable to jq.

`test-worker.mjs` runs the actual `worker.js` in a Node Worker with adapters for
browser messaging and fetching local WASM. It checks streaming before an
infinite continuation completes, termination, restart and repeated requests.
It does not replace a browser integration test. Chromium validation in the
development container was blocked by missing browser system libraries.
