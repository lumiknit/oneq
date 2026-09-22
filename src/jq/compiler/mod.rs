//! Internal compilation pipeline; callers append through Session.
pub mod analyze;
pub mod calc;
mod containers;
mod emit;
mod ir_const_fold;
mod lower;
pub mod modules;
pub(crate) mod rewrite;
pub(crate) mod templates;

use super::{symbols::Symbols, vm::code::Chunk};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct CompileOptions {
    pub path: String,
    pub module_dirs: Vec<PathBuf>,
    pub repl: bool,
    pub calc: bool,
    /// Expand closed, non-recursive function templates before emission.
    pub inline: bool,
    /// Whether this is the top-level program rather than an imported module;
    /// `$__loc__` reports `<top-level>` here, matching jq, instead of `path`.
    pub entry: bool,
}
impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            path: "<main>".into(),
            module_dirs: vec![],
            repl: false,
            calc: false,
            inline: true,
            entry: true,
        }
    }
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("{0}")]
pub struct CompileError(pub String);

/// Work on private state. Session publishes only after every phase succeeds.
pub(crate) fn compile(
    graph: &modules::ModuleGraph,
    options: &CompileOptions,
    symbols: &mut Symbols,
    templates: &templates::TemplateStore,
) -> Result<(Chunk, templates::TemplateStore), CompileError> {
    modules::link::compile(graph, options, symbols, templates)
}
