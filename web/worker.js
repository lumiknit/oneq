import init, { build_configuration, prepare, resume, cancel } from "./pkg/oneq.js";

const ready = init();
ready.then(() => {
  self.postMessage({ type: "build-configuration", value: build_configuration() });
});
let stopped = false;
let pumping = false;
function pump() {
  if (stopped || pumping) return;
  pumping = true;
  try {
    const code = resume(10000);
    if (code === 0 || code === 1) setTimeout(() => { pumping = false; pump(); }, 0);
    else { self.postMessage({ type: "done", code }); pumping = false; }
  } catch (error) { pumping = false; self.postMessage({ type: "failure", message: String(error) }); }
}
self.onmessage = async ({ data }) => {
  try {
    await ready;
    if (data?.type === "start") {
      stopped = false;
      const code = prepare(data.packet);
      if (code !== 0) { self.postMessage({ type: "done", code }); return; }
      pump();
    } else if (data?.type === "cancel") {
      stopped = true; cancel(); self.postMessage({ type: "cancelled" });
    } else { stopped = false; const code = prepare(data); if (code === 0) pump(); else self.postMessage({ type: "done", code }); }
  } catch (error) {
    // Traps are reported separately from CLI exit codes.
    self.postMessage({ type: "failure", message: String(error) });
  }
};
