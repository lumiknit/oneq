//! Append is atomic; executions exclusively borrow the session and commit on EOF.
use super::{InputMode, JqError, RunOutcome, Vm, VmEvent, code::Program, frame::Frame, host::Host};
use crate::jq::{
    compiler::{
        self, CompileError, CompileOptions,
        modules::{ModuleCache, ModuleGraph},
        templates::TemplateStore,
    },
    ir::{BindingId, CallTarget, FunctionId},
    symbols::{Names, Symbols},
};
use crate::{data::Value, strs};
use std::{
    collections::HashMap,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntryId {
    owner: u64,
    index: usize,
}

#[derive(Debug)]
pub struct Checkpoint {
    owner: u64,
    entries: usize,
    names: Names,
    values: HashMap<BindingId, Value>,
    functions: HashMap<FunctionId, Rc<super::frame::Closure>>,
}
#[derive(Debug)]
pub struct Session {
    owner: u64,
    program: Program,
    modules: ModuleCache,
    templates: TemplateStore,
    symbols: Symbols,
    values: HashMap<BindingId, Value>,
    functions: HashMap<FunctionId, Rc<super::frame::Closure>>,
    entries: Vec<Option<usize>>,
}
impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}
/// The jq-language prelude (jq's own builtin.jq), loaded once into every fresh session.
const BUILTIN_JQ: &str = include_str!("../builtins/builtin.jq");

struct NoHost;
impl Host for NoHost {
    fn next_input(&mut self) -> Option<Result<Value, JqError>> {
        None
    }
    fn environment(&mut self) -> Result<Value, JqError> {
        Ok(Value::Null)
    }
}

impl Session {
    pub fn new() -> Self {
        let mut session = Self {
            owner: NEXT_SESSION.fetch_add(1, Ordering::Relaxed),
            program: Program::default(),
            modules: ModuleCache::default(),
            templates: TemplateStore::default(),
            symbols: Symbols::default(),
            values: HashMap::new(),
            functions: HashMap::new(),
            entries: vec![],
        };
        session
            .load_builtin()
            .expect("embedded builtin.jq prelude must compile and run");
        session
    }
    /// Publish the jq-language prelude's definitions so later entries can call them by name.
    /// Compiling alone only declares the names; each def's closure is only registered once
    /// its `Define` instruction actually runs, so the entry has to run to completion too.
    fn load_builtin(&mut self) -> Result<(), JqError> {
        let mut host = NoHost;
        for (path, source) in [
            ("<builtin.jq>", BUILTIN_JQ),
            ("<compat.jq>", include_str!("../builtins/compat.jq")),
        ] {
            let entry = self
                .append(
                    source,
                    CompileOptions {
                        path: path.into(),
                        repl: true,
                        ..CompileOptions::default()
                    },
                )
                .map_err(|e| JqError::InvalidCode(e.to_string()))?;
            for result in self.run(entry, &mut host, InputMode::Null)? {
                result?;
            }
        }
        Ok(())
    }
    /// Inject a CLI argument as a fresh declaration. Old entries keep the old ID.
    pub fn bind(&mut self, name: &str, value: Value) {
        let id = self.symbols.declare(strs::intern(name), None);
        self.values.insert(id, value);
    }
    /// Parse the entry and all imports without requiring IR lowering to be implemented.
    /// A complete dependency graph publishes its immutable file cache even if later
    /// lowering fails. Failed loading publishes nothing. No names or code are published.
    pub fn prepare(
        &mut self,
        source: &str,
        options: &CompileOptions,
    ) -> Result<ModuleGraph, CompileError> {
        self.modules.prepare(source, options)
    }
    pub fn append(
        &mut self,
        source: &str,
        options: CompileOptions,
    ) -> Result<EntryId, CompileError> {
        let graph = self.prepare(source, &options)?;
        let mut symbols = self.symbols.clone();
        // A compiled definition may not have executed yet. Inlining must not
        // turn an uninitialized REPL function into an executable one.
        let available = self
            .templates
            .iter()
            .filter(|(id, _)| self.functions.contains_key(id))
            .map(|(id, template)| (*id, template.clone()))
            .collect();
        let (mut chunk, templates) = compiler::compile(&graph, &options, &mut symbols, &available)?;
        for function in &mut chunk.functions {
            function.code.chunk = self.program.chunks.len();
        }
        for binding in &chunk.ir.export_bindings {
            self.values.entry(*binding).or_insert(Value::Null);
        }
        self.values.extend(chunk.data_bindings.iter().cloned());
        let index = self.entries.len();
        self.entries.push(Some(self.program.chunks.len()));
        self.program.chunks.push(chunk);
        self.symbols = symbols;
        self.templates.extend(templates);
        Ok(EntryId {
            owner: self.owner,
            index,
        })
    }
    pub fn is_definition(&self, entry: EntryId) -> Result<bool, JqError> {
        Ok(self.program.chunks[self.chunk(entry)?].definition_only)
    }

    /// Names of functions currently callable: declared and with a live closure
    /// (a `def` whose body has actually run once to register it).
    pub fn function_names(&self) -> impl Iterator<Item = (&'static str, usize)> + '_ {
        self.symbols
            .names
            .functions
            .iter()
            .filter_map(|((name, arity), target)| match target {
                CallTarget::Function(id) if self.functions.contains_key(id) => {
                    strs::resolve(*name).map(|name| (name, *arity))
                }
                _ => None,
            })
    }

    /// Names of variables currently readable: declared and materialized into
    /// session state (values only persist across entries when exported).
    pub fn variable_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.symbols
            .names
            .bindings
            .iter()
            .filter_map(|(name, id)| self.values.contains_key(id).then(|| strs::resolve(*name))?)
    }

    /// Whether this entry can request another value from the host input
    /// stream (for example through `input`/`inputs`).
    pub fn requires_host_input(&self, entry: EntryId) -> Result<bool, JqError> {
        let chunk = &self.program.chunks[self.chunk(entry)?];
        let summary = compiler::analyze::analyze_summary(&chunk.ir, chunk.entry);
        Ok(summary.observations.may_add_input || !summary.finite)
    }
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            owner: self.owner,
            entries: self.entries.len(),
            names: self.symbols.names.clone(),
            values: self.values.clone(),
            functions: self.functions.clone(),
        }
    }
    pub fn rollback(&mut self, checkpoint: Checkpoint) -> Result<(), JqError> {
        if checkpoint.owner != self.owner {
            return Err(JqError::InvalidEntry);
        }
        for entry in &mut self.entries[checkpoint.entries..] {
            *entry = None;
        }
        self.symbols.names = checkpoint.names;
        self.values = checkpoint.values;
        self.functions = checkpoint.functions;
        // Do not truncate declaration/code/entry arenas: IDs must never be reused.
        Ok(())
    }
    fn chunk(&self, entry: EntryId) -> Result<usize, JqError> {
        if entry.owner != self.owner {
            return Err(JqError::InvalidEntry);
        }
        self.entries
            .get(entry.index)
            .copied()
            .flatten()
            .ok_or(JqError::InvalidEntry)
    }
    pub fn run<'a>(
        &'a mut self,
        entry: EntryId,
        host: &'a mut dyn Host,
        mode: InputMode,
    ) -> Result<Execution<'a>, JqError> {
        let chunk = self.chunk(entry)?;
        let mode = if self.program.chunks[chunk].definition_only {
            InputMode::Null
        } else {
            mode
        };
        let candidate = self.values.clone();
        let candidate_functions = self.functions.clone();
        Ok(Execution {
            session: self,
            host,
            chunk,
            mode,
            started: false,
            vm: None,
            candidate,
            candidate_functions,
            outcome: None,
            continue_after_error: false,
            input_error: false,
            last_output: None,
        })
    }
    /// Deterministic arena order makes this useful before a pretty IR printer exists.
    pub fn dump(&self, entry: EntryId) -> Result<String, JqError> {
        let chunk = &self.program.chunks[self.chunk(entry)?];
        Ok(chunk
            .ir
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| format!("e{i}: {:?}\n", n.expr))
            .collect())
    }

    /// Render only the optimized entry expression, without the arena/function
    /// metadata. This is intended for compiler diagnostics such as `--dump-ir`.
    pub fn dump_expr(&self, entry: EntryId) -> Result<String, JqError> {
        let chunk = &self.program.chunks[self.chunk(entry)?];
        let mut out = String::new();
        use std::fmt::Write as _;
        writeln!(out, "root: e{}", chunk.entry.0).expect("writing to String cannot fail");
        for item in crate::jq::compiler::rewrite::postorder(&chunk.ir, chunk.entry) {
            if let crate::jq::compiler::rewrite::Item::Expr(id) = item {
                match &chunk.ir.nodes[id.0].expr {
                    crate::jq::ir::Expr::BuiltinCall { builtin, args } => {
                        let spec = crate::jq::builtins::spec(*builtin);
                        writeln!(
                            out,
                            "e{}: BuiltinCall {{ builtin: BuiltinId({}, \"{}\"), args: {:?} }}",
                            id.0, builtin.0, spec.name, args
                        )
                    }
                    expr => writeln!(out, "e{}: {:?}", id.0, expr),
                }
                .expect("writing to String cannot fail");
            }
        }
        Ok(out)
    }
}

pub struct Execution<'a> {
    session: &'a mut Session,
    host: &'a mut dyn Host,
    chunk: usize,
    mode: InputMode,
    started: bool,
    vm: Option<Vm>,
    candidate: HashMap<BindingId, Value>,
    candidate_functions: HashMap<FunctionId, Rc<super::frame::Closure>>,
    outcome: Option<RunOutcome>,
    continue_after_error: bool,
    input_error: bool,
    last_output: Option<bool>,
}
impl Execution<'_> {
    /// CLI policy: a runtime error ends this input, then the next input starts.
    /// The REPL retains the default transaction-wide error behavior.
    pub fn continue_after_error(&mut self) {
        self.continue_after_error = true;
    }
    pub fn input_status(&self) -> (bool, Option<bool>) {
        (self.input_error, self.last_output)
    }
    pub fn outcome(&self) -> Option<&RunOutcome> {
        self.outcome.as_ref()
    }
    pub fn cancel(&mut self) {
        if self.outcome.is_none() {
            self.vm = None;
            self.outcome = Some(RunOutcome::Cancelled);
        }
    }
    /// Each input advance and VM instruction consumes budget. Suspension is not EOF.
    pub fn resume(&mut self, budget: usize) -> VmEvent {
        self.resume_mode::<true>(budget)
    }

    pub(crate) fn resume_mode<const LIMITED: bool>(&mut self, mut budget: usize) -> VmEvent {
        if self.outcome.is_some() {
            return VmEvent::Done;
        }
        while !LIMITED || budget > 0 {
            if let Some(vm) = &mut self.vm {
                let event = vm.resume::<LIMITED>(&self.session.program, self.host, &mut budget);
                self.candidate
                    .extend(vm.exports.iter().map(|(id, value)| (*id, value.clone())));
                self.candidate_functions.extend(
                    vm.exported_functions
                        .iter()
                        .map(|(id, value)| (*id, value.clone())),
                );
                match event {
                    VmEvent::Done => self.vm = None,
                    VmEvent::Error(error) => {
                        self.vm = None;
                        self.input_error = true;
                        if !self.continue_after_error || matches!(error, JqError::Input(_)) {
                            self.outcome = Some(RunOutcome::Error);
                        }
                        self.candidate = self.session.values.clone();
                        self.candidate_functions = self.session.functions.clone();
                        return VmEvent::Error(error);
                    }
                    VmEvent::Output(value) => {
                        self.last_output = Some(value.is_truthy());
                        return VmEvent::Output(value);
                    }
                    VmEvent::Halt { code, value } => {
                        self.vm = None;
                        self.outcome = Some(RunOutcome::Halt {
                            code,
                            value: value.clone(),
                        });
                        return VmEvent::Halt { code, value };
                    }
                    event => return event,
                }
            } else {
                if LIMITED {
                    budget -= 1;
                }
                let next = match self.mode {
                    InputMode::Host => self.host.next_input(),
                    InputMode::Null if !self.started => Some(Ok(Value::Null)),
                    InputMode::Null => None,
                };
                self.started = true;
                match next {
                    Some(Ok(input)) => {
                        self.input_error = false;
                        self.vm = Some(Vm::start(
                            self.chunk,
                            input,
                            Frame {
                                base: 0,
                                slots: self.session.program.chunks[self.chunk]
                                    .slots
                                    .iter()
                                    .map(|key| {
                                        use super::{code::SlotKey, frame::SlotValue};
                                        match key {
                                            SlotKey::Local(id) => {
                                                self.candidate.get(id).map(|value| {
                                                    SlotValue::Local(value.clone().into())
                                                })
                                            }
                                            SlotKey::Function(id) => self
                                                .candidate_functions
                                                .get(id)
                                                .map(|closure| SlotValue::Filter(closure.clone())),
                                            SlotKey::Filter(_) => None,
                                        }
                                    })
                                    .collect(),
                                parent: None,
                            },
                        ))
                    }
                    Some(Err(error)) => {
                        self.outcome = Some(RunOutcome::Error);
                        return VmEvent::Error(error);
                    }
                    None => {
                        self.session.values = std::mem::take(&mut self.candidate);
                        self.session.functions = std::mem::take(&mut self.candidate_functions);
                        self.outcome = Some(RunOutcome::Complete);
                        return VmEvent::Done;
                    }
                }
            }
        }
        VmEvent::Suspended
    }
}
impl Iterator for Execution<'_> {
    type Item = Result<Value, JqError>;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.resume(4096) {
                VmEvent::Output(value) => return Some(Ok(value)),
                VmEvent::Error(error) => return Some(Err(error)),
                VmEvent::Done | VmEvent::Halt { .. } => return None,
                VmEvent::Suspended => {}
            }
        }
    }
}
// Dropping Execution drops its uncommitted candidate, including after a yield.
