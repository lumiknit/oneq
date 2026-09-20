// A chunk may split UTF-8 or ANSI sequences. The frontend must decode each
// channel incrementally. Copy before WASM reuses or grows its linear memory.
export function write(channel, bytes) {
  const chunk = bytes.slice();
  globalThis.postMessage({ type: "output", channel, bytes: chunk }, [chunk.buffer]);
}
