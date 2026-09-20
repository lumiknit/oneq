//! jq syntax and the shared incremental compiler/VM session.
pub mod builtins;
pub mod compiler;
pub mod ir;
pub mod parser;
pub mod symbols;
pub mod vm;

pub use compiler::CompileOptions;
pub use vm::session::{EntryId, Session};
