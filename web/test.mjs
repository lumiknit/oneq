// Run after web/build.sh and cargo build --bin 1q. No npm dependencies.
import assert from "node:assert/strict";
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import init, { run } from "./pkg/oneq.js";

process.chdir(fileURLToPath(new URL("..", import.meta.url)));
const binary = process.env.ONEQ_BIN ?? "target/debug/1q";
await init({ module_or_path: readFileSync(new URL("./pkg/oneq_bg.wasm", import.meta.url)) });
let events;
globalThis.postMessage = (event) => events.push(event);

function packet(argv, input = "") {
  return Buffer.concat([Buffer.from(JSON.stringify(argv)), Buffer.from([0]), Buffer.from(input)]);
}
function execute(bytes) {
  events = [];
  const code = run(bytes);
  return {
    code,
    stdout: Buffer.concat(events.filter(e => e.channel === 1).map(e => Buffer.from(e.bytes))),
    stderr: Buffer.concat(events.filter(e => e.channel === 2).map(e => Buffer.from(e.bytes))),
    events,
  };
}
let passed = 0;
function parity(name, argv, input = "") {
  const expected = spawnSync(binary, argv, { input, maxBuffer: 16 * 1024 * 1024 });
  assert.ifError(expected.error);
  const actual = execute(packet(argv, input));
  assert.equal(actual.code, expected.status, `${name}: exit code: ${actual.stderr}`);
  assert.deepEqual(actual.stdout, expected.stdout, `${name}: stdout`);
  assert.deepEqual(actual.stderr, expected.stderr, `${name}: stderr`);
  passed++;
  return actual;
}

for (const [name, argv, input] of [
  ["identity", ["-c", "."], '{"hello":"한글😀"}\n2'],
  ["raw bytes", ["-Rsc", "."], Buffer.from([65, 0, 66, 13, 10, 0xff])],
  ["NUL output", ["--raw-output0", ".[]"], '["a","한글"]'],
  ["raw joined", ["-jr", ".[]"], '["a","b"]'],
  ["slurp", ["-sc", "add"], "1\n2\n3"],
  ["shared inputs", ["-nc", "[inputs]"], "1\n2\n3"],
  ["stream", ["--stream", "-c", "."], '{"a":[1,2]}'],
  ["stream errors", ["--stream-errors", "-c", "."], '[1,broken'],
  ["named args", ["-nc", "--arg", "x", "hello world\n한글", "--argjson", "y", "[1,2]", "[$x,$y,$ARGS.named]"], ""],
  ["positional args", ["-nc", "--args", "$ARGS.positional", "hello world", "--file"], ""],
  ["JSON args", ["-nc", "--jsonargs", "$ARGS.positional", "1", '{"a":2}'], ""],
  ["color", ["-C", "."], '{"a":[true,null,"한글"]}'],
  ["monochrome wins", ["-C", "-M", "."], '{"a":1}'],
  ["format", ["--fmt"], '{foo: [1,2], bar: true}'],
  ["format identity", ["--fmt", "."], '{foo:1}'],
  ["format colors", ["--fmt", "-C"], '{foo:1}'],
  ["format error", ["--fmt"], '{foo:'],
  ["YAML", ["-F", "yaml", "-c", "."], 'a: 1\nb: [2, 3]\n'],
  ["TOML output", ["-T", "toml", "."], '{"a":1}'],
  ["document input", ["--doc", "-c", "type"], ""],
  ["stderr builtins", ["-nc", '"한글" | debug | stderr'], ""],
  ["partial output", ["-nc", '1,error("bad"),2'], ""],
  ["parse error", ["-c", "."], '1\n[broken'],
  ["compile error", ["-nc", "missing_filter"], ""],
  ["exit false", ["-nec", "false"], ""],
  ["exit empty", ["-nec", "empty"], ""],
  ["halt", ["-nc", '"stop\n" | halt_error(7)'], ""],
  ["help", ["--help"], ""],
  ["invalid flag", ["--bad-flag"], ""],
  ["invalid format", ["-F", "no-such-format", "."], ""],
]) parity(name, argv, input);

// Assert bytes reach the host in execution order, before the call returns.
const streaming = parity("streaming order", ["-nc", '1,("notice"|debug|empty),2']);
assert.deepEqual(streaming.events.map(e => e.channel).filter((c, i, a) => i === 0 || c !== a[i - 1]), [1, 2, 1]);
assert.match(execute(packet(["-Cn", "1"])).stdout.toString(), /\x1b\[/);

for (const badPacket of [
  Buffer.from("[]"), Buffer.from('{}\0'), Buffer.from('[1]\0'),
  Buffer.from('[".\\u0000"]\0'), Buffer.from('[] []\0'),
  Buffer.from([0xff, 0]), Buffer.from('["."\0'),
]) {
  const result = execute(badPacket);
  assert.equal(result.code, 2);
  assert.equal(result.stdout.length, 0);
  assert.ok(result.stderr.length > 0);
  passed++;
}
for (const argv of [
  ["-f", "missing.jq"], ["-L", ".", "."], ["-i", "."],
  ["--rawfile", "x", "missing", "."], ["--slurpfile", "x", "missing", "."],
  [".", "missing.json"], ["--fmt", "missing.jq"], ["--with"], ["--repl"],
]) {
  const result = execute(packet(argv));
  assert.equal(result.code, 2, argv.join(" "));
  assert.match(result.stderr.toString(), /not supported in WASM/);
  passed++;
}
for (const filter of ['import "missing" as m; .', 'include "missing"; .']) {
  const result = execute(packet(["-n", filter]));
  assert.equal(result.code, 3);
  assert.match(result.stderr.toString(), /import\/include are not supported/);
  passed++;
}
assert.equal(execute(packet(["-nc", "[$ENV,env]"])).stdout.toString(), "[{},{}]\n");
const now = execute(packet(["-n", "now"]));
assert.equal(now.code, 0);
assert.ok(Math.abs(Number(now.stdout.toString()) - Date.now() / 1000) < 10);
passed += 2;

// Reuse every file-free jq-oracle case. Fixtures remain the source of truth;
// native parity here complements their real-jq comparison in Rust tests.
const dir = "tests/fixtures/jq-oracle";
let fixtures = 0;
for (const name of readdirSync(dir).sort()) {
  if (!name.endsWith(".jq") || name.startsWith("_")) continue;
  const filter = readFileSync(`${dir}/${name}`, "utf8");
  if (/^\s*(?:import|include)\s/m.test(filter)) continue;
  const first = filter.split("\n")[0];
  // All current file-free fixture shebangs are '-n'. Fail explicitly if a new
  // convention needs supporting, rather than mis-tokenizing quoted arguments.
  if (first.startsWith("#!")) assert.equal(first.trim(), "#! -n", name);
  const argv = ["-c", ...(first.startsWith("#!") ? ["-n"] : []), filter];
  const inputPath = `${dir}/${name.replace(/\.jq$/, ".in.jsonl")}`;
  parity(name, argv, existsSync(inputPath) ? readFileSync(inputPath) : Buffer.alloc(0));
  fixtures++;
}
parity("fresh call after failures", ["-nc", "1+2"]);
console.log(`${passed} checks passed (${fixtures} file-free jq-oracle fixtures).`);
