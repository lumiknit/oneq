//! Integration tests: build a `Session` (or spawn the CLI) and check
//! the output of a full run, often against real `jq` as an oracle.
//! Tests that only exercise one internal function/module directly
//! belong in `tests/unit.rs` instead.
#[path = "common/mod.rs"]
mod common;
#[path = "integration/compile.rs"]
mod compile;

#[path = "integration/builtins.rs"]
mod builtins;
#[path = "integration/cli_stack.rs"]
mod cli_stack;
#[path = "integration/fmt.rs"]
mod fmt;
#[path = "integration/jq_oracle.rs"]
mod jq_oracle;
#[path = "integration/modules.rs"]
mod modules;
#[path = "integration/session.rs"]
mod session;
#[path = "integration/vm_features.rs"]
mod vm_features;

#[path = "jq-test/mod.rs"]
mod jq_test;
