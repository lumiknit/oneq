mod input;
mod output;

pub use input::{Input, InputTracker, SharedInputTracker};
pub use output::Output;

// Keep both CLI diagnostics and builtin side effects on the same host streams.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub(crate) use std::io::{stderr, stdout};

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn stdout() -> impl std::io::Write {
    crate::lib_wasm::Stream(1)
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn stderr() -> impl std::io::Write {
    crate::lib_wasm::Stream(2)
}
