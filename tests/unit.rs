//! Unit tests: exercise a specific internal function/module directly
//! (parser, compiler, data format) without running a full program
//! through `Session` and comparing its output.
#[path = "unit/compiler.rs"]
mod compiler;
#[path = "unit/data/mod.rs"]
mod data;
#[path = "unit/parser.rs"]
mod parser;
