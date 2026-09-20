# 1q playground

Minimal Solid + Vite frontend for the WASM runner in `../web/`.

```sh
# First build (or after changing Rust): requires Rust's wasm32-unknown-unknown
# target and the wasm-bindgen CLI version specified in ../web/README.md.
npm run build:wasm

npm run dev
npm run build
npm run preview
```

The same package scripts can be run with Deno. The existing `deno.lock` is
retained; no extra frontend dependencies are needed.

Vite follows the existing `../web/worker.js` module and bundles its JS helpers
and WASM asset into `dist/`. No manual copying or separate `web/` deployment is
needed. `base: './'` makes the production asset URLs relative, including the
Worker and WASM, so the directory can be served under a subpath. Serve `dist/`
over HTTP, with `.wasm` served as `application/wasm`.

Enter flags in Options (for example `-c --arg name "hello world"`), the jq
filter in its textarea, and input in stdin. Options support single/double
quotes and backslash escapes, but no shell expansion. With `--fmt`, stdin is
formatted and the jq field is ignored.

The jq editor also has a Format button with Pretty, Oneline and Compact modes.
It formats the editor contents through WASM, independently of Options and
stdin, and replaces the code only on success. Errors appear in stderr; stopping
or failing formatting preserves the original code. Formatting output is also
shown in stdout, without ANSI escapes.

Run creates a Worker, streams stdout/stderr separately, and shows its exit
code. Stop terminates the Worker; the next run starts a fresh one. Ctrl/Cmd +
Enter also runs. UTF-8 is decoded incrementally per channel; updates are
batched per animation frame. Output is plain text: ANSI escape sequences are
preserved but are not interpreted as colors in this initial UI.

Browser behavior is left for manual verification; the build performs the
TypeScript and production bundling checks.
