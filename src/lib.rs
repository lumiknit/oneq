pub mod cmd;
pub mod data;
pub mod doc;
pub mod io;
pub mod jq;
pub mod render;
pub mod strs;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod lib_wasm;

use std::fmt;

pub struct BuildConfig;

impl fmt::Display for BuildConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "pkg_name {}", env!("CARGO_PKG_NAME"))?;
        writeln!(f, "pkg_version {}", env!("CARGO_PKG_VERSION"))?;
        writeln!(f, "rustc {}", env!("RUSTC_VERSION"))?;
        writeln!(f, "target {}", env!("BUILD_TARGET"))?;
        writeln!(f, "opt_level {}", env!("BUILD_OPT_LEVEL"))?;
        writeln!(f, "debug {}", env!("BUILD_DEBUG"))?;
        write!(f, "profile {}", env!("BUILD_PROFILE"))
    }
}
