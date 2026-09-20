// Exercise the actual browser Worker module using Node's Worker with a small
// Web Worker API adapter. Browser rendering/HTTP integration is not tested here.
import assert from "node:assert/strict";
import { Worker } from "node:worker_threads";

const bootstrap = `
  import { parentPort, workerData } from 'node:worker_threads';
  import { readFile } from 'node:fs/promises';
  globalThis.self = globalThis;
  globalThis.postMessage = (message, transfer) => parentPort.postMessage(message, transfer);
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (url, options) => {
    if (String(url).startsWith('file:')) {
      return new Response(await readFile(new URL(url)), { headers: { 'Content-Type': 'application/wasm' } });
    }
    return originalFetch(url, options);
  };
  await import(workerData);
  parentPort.on('message', data => self.onmessage({ data }));
`;
function makeWorker() {
  return new Worker(new URL(`data:text/javascript,${encodeURIComponent(bootstrap)}`), {
    workerData: new URL("./worker.js", import.meta.url).href,
  });
}
function request(worker, argv, stdin = "", stopOnOutput = false) {
  return new Promise((resolve, reject) => {
    const events = [];
    const timer = setTimeout(() => finish(new Error("Worker request timed out")), 15000);
    function finish(error, value) {
      clearTimeout(timer);
      worker.off("message", receive);
      worker.off("error", fail);
      if (error) reject(error); else resolve(value);
    }
    function fail(error) { finish(error); }
    function receive(event) {
      events.push(event);
      if (event.type === "failure") finish(new Error(event.message));
      else if (event.type === "done" || (stopOnOutput && event.type === "output")) finish(null, events);
    }
    worker.on("message", receive);
    worker.on("error", fail);
    const bytes = new Uint8Array(Buffer.concat([
      Buffer.from(JSON.stringify(argv)), Buffer.from([0]), Buffer.from(stdin),
    ]));
    worker.postMessage(bytes, [bytes.buffer]);
    assert.equal(bytes.byteLength, 0, "request bytes must transfer to Worker");
  });
}
function text(events, channel) {
  return Buffer.concat(events.filter(e => e.channel === channel).map(e => Buffer.from(e.bytes))).toString();
}
let worker = makeWorker();
try {
  const normal = await request(worker, ["-c", ".[]"], '["한글",2]');
  assert.match(normal.find(e => e.type === "build-configuration")?.value ?? "", /pkg_version /);
  assert.equal(text(normal, 1), '"한글"\n2\n');
  assert.equal(normal.at(-1).code, 0);
  const error = await request(worker, ["-nc", '1,error("bad")']);
  assert.equal(text(error, 1), "1\n");
  assert.match(text(error, 2), /bad/);
  assert.equal(error.at(-1).code, 5);
  const fmt = await request(worker, ["--fmt", "-C"], '{foo:1}');
  assert.equal(fmt.at(-1).code, 0);
  assert.match(text(fmt, 1), /\x1b\[/);
  // First output must arrive before an infinite continuation completes.
  const early = await request(worker, ["-nc", "1, (while(true; .) | empty)"], "", true);
  assert.equal(text(early, 1), "1\n");
  assert.ok(!early.some(e => e.type === "done"));
} finally {
  await worker.terminate();
}
worker = makeWorker();
try {
  const fresh = await request(worker, ["-nc", "42"]);
  assert.equal(text(fresh, 1), "42\n");
  assert.equal(fresh.at(-1).code, 0);
} finally {
  await worker.terminate();
}
console.log("Worker module: streaming, repeated calls, errors, colors/formatting, cancellation and restart passed.");
