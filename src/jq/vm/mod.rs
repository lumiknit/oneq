mod builtin;
pub mod code;
pub mod frame;
pub mod host;
pub mod session;
mod stack;
use stack::Stack;
pub(crate) mod value;

use crate::jq::ir::{BindingId, FunctionId};
use crate::{data::Value, jq::compiler::calc::CalcContext, strs};
use frame::{ChoicePoint, Closure, Frame, Handler, Operand, ReturnFrame, SlotValue};
use host::Host;
use std::{collections::HashMap, rc::Rc};

#[derive(Clone, Debug, thiserror::Error)]
pub enum JqError {
    #[error("{0}")]
    Runtime(Value),
    #[error("input: {0}")]
    Input(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("invalid entry (foreign session or rolled back)")]
    InvalidEntry,
    #[error("uninitialized binding")]
    Uninitialized,
    #[error("invalid bytecode: {0}")]
    InvalidCode(String),
}

#[cfg(test)]
mod recovery_tests {
    use super::*;

    #[test]
    fn frame_reuse_does_not_mutate_saved_bindings() {
        let mut vm = Vm::start(0, Value::Null, Frame::default());
        let root = vm.snapshot(0);
        vm.bind(0, SlotValue::Local(Value::Float(1.0).into()));
        let saved = vm.snapshot(1);
        vm.bind(0, SlotValue::Local(Value::Float(2.0).into()));
        let reusable = Rc::as_ptr(&vm.frame);
        vm.restore(root);
        assert_eq!(vm.spare_frames.len(), 1);
        vm.bind(0, SlotValue::Local(Value::Float(3.0).into()));
        assert_eq!(Rc::as_ptr(&vm.frame), reusable);
        vm.restore(saved);
        assert!(matches!(
            vm.frame.get(0),
            Some(SlotValue::Local(Operand {
                value: Value::Float(1.0),
                ..
            }))
        ));
        assert!(
            vm.spare_frames
                .iter()
                .all(|frame| frame.slots.is_empty() && frame.parent.is_none())
        );
    }

    #[test]
    fn iteration_keeps_only_one_pending_choice_and_skips_lost_paths() {
        let input = Value::Array(Rc::new(
            (0..100_000).map(|i| Value::Float(i as f64)).collect(),
        ));
        let mut vm = Vm::start(0, input, Frame::default());
        // Value-producing instructions can invalidate the current path.
        vm.input.path = None;
        vm.advance_iteration(0).unwrap();
        for index in 1..100_000 {
            assert_eq!(vm.choices.len(), 1);
            assert!(vm.input.path.is_none());
            vm.backtrack();
            let next = vm.iteration.take().unwrap();
            assert_eq!(next, index);
            vm.advance_iteration(next).unwrap();
        }
        assert!(vm.choices.is_empty());
        assert_eq!(vm.input.value, Value::Float(99_999.0));
    }
}
impl JqError {
    /// Renders the way jq's own CLI does for `error(...)`/uncaught runtime
    /// errors: a string payload prints as its raw text (no JSON quoting),
    /// matching `jq: error (at <stdin>:1): boom` for `error("boom")`; any
    /// other jq value prints as `(not a string): <value>`, matching jq's
    /// `jq: error (at <stdin>:1) (not a string): 42` for `error(42)`.
    pub fn user_message(&self) -> String {
        match self {
            JqError::Runtime(Value::String(s)) => s.to_string(),
            JqError::Runtime(value) => format!("(not a string): {value}"),
            other => other.to_string(),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum RunOutcome {
    Complete,
    Error,
    Halt { code: i32, value: Value },
    Cancelled,
}
pub enum VmEvent {
    Output(Value),
    Done,
    Error(JqError),
    Halt { code: i32, value: Value },
    Suspended,
}
#[derive(Clone, Copy, Debug)]
pub enum InputMode {
    Host,
    Null,
}

/// All mutable execution data lives here, never in Program or a function body.
#[derive(Debug, Default)]
pub(crate) struct Vm {
    pc: usize,
    chunk: usize,
    calls: Stack<ReturnFrame>,
    handlers: Stack<Handler, 1>,
    folds: Stack<Rc<std::cell::RefCell<Value>>>,
    labels: Stack<(crate::jq::ir::LabelId, Handler), 1>,
    native: Option<crate::jq::builtins::NativeState>,
    iteration: Option<usize>,
    alternatives: Stack<Rc<std::cell::Cell<bool>>>,
    pub(crate) exports: HashMap<BindingId, Value>,
    pub(crate) exported_functions: HashMap<FunctionId, Rc<Closure>>,
    input: Operand,
    frame: Rc<Frame>,
    /// Only unpublished, uniquely owned frames enter this bounded reuse pool.
    /// It is allocation scratch space, never part of a continuation.
    spare_frames: Vec<Rc<Frame>>,
    choices: Vec<ChoicePoint>,
    operands: Stack<Operand>,
    /// Nesting depth of `BeginPath`/`EndPath` (i.e. `path(...)`, `=`, `|=`, ...).
    /// Outside any such context a lost path is silently harmless (the value
    /// keeps evaluating normally), but inside one it must surface jq's
    /// specific "Invalid path expression near attempt to ..." error the
    /// moment `Index`/`Slice`/`Iterate` tries to extend an already-lost path,
    /// matching real jq's separate path-vs-value compilation without
    /// actually needing two compiled forms of the same query.
    path_depth: usize,
    /// Saved `path_depth` values from `SuspendPath`/`ResumePath` bracketing
    /// (dynamic index/slice key sub-expressions inside a `path(...)`).
    path_depth_stack: Stack<usize>,
    calc: CalcContext,
    /// LIFO accumulators for nested Array literals (BeginCollect/EndCollect).
    collect: Vec<Vec<Value>>,
    done: bool,
}
impl Vm {
    fn new_frame(&mut self, base: usize, len: usize, parent: Rc<Frame>) -> Rc<Frame> {
        let mut frame = self.spare_frames.pop().unwrap_or_default();
        let region = Rc::get_mut(&mut frame).expect("spare frame is unique");
        region.base = base;
        region.slots.resize_with(len, || None);
        region.parent = Some(parent);
        frame
    }
    fn recycle_frame(&mut self, mut frame: Rc<Frame>) {
        // Never change frames retained by a closure, choice or return address.
        while let Some(region) = Rc::get_mut(&mut frame) {
            let parent = region.parent.take();
            region.slots.clear();
            if self.spare_frames.len() < 32 {
                self.spare_frames.push(frame);
            }
            match parent {
                Some(next) => frame = next,
                None => break,
            }
        }
    }
    fn bind(&mut self, slot: usize, value: SlotValue) {
        let mut frame = self.new_frame(slot, 1, self.frame.clone());
        Rc::get_mut(&mut frame).unwrap().slots[0] = Some(value);
        self.frame = frame;
    }
    pub(crate) fn start(chunk: usize, input: Value, frame: Frame) -> Self {
        Self {
            chunk,
            input: Operand {
                value: input,
                path: Some(Stack::default()),
            },
            frame: Rc::new(frame),
            ..Self::default()
        }
    }
    fn snapshot(&self, pc: usize) -> ChoicePoint {
        ChoicePoint {
            pc,
            chunk: self.chunk,
            calls: self.calls.clone(),
            handlers: self.handlers.clone(),
            alternatives: self.alternatives.clone(),
            skip_if: None,
            folds: self.folds.clone(),
            labels: self.labels.clone(),
            native: self.native.clone(),
            iteration: self.iteration,
            input: self.input.clone(),
            frame: self.frame.clone(),
            operands: self.operands.clone(),
            path_depth: self.path_depth,
            path_depth_stack: self.path_depth_stack.clone(),
        }
    }
    fn advance_native(
        &mut self,
        mut state: crate::jq::builtins::NativeState,
    ) -> Result<(), JqError> {
        use crate::jq::builtins::NativeEvent;
        match state.resume(None)? {
            NativeEvent::Output(value) => {
                let mut point = self.snapshot(self.pc);
                point.native = Some(state);
                self.choices.push(point);
                self.input.value = value;
            }
            NativeEvent::Done => self.backtrack(),
            NativeEvent::Callback { .. } => {
                return Err(JqError::Unsupported("native filter callback".into()));
            }
        }
        Ok(())
    }
    /// Keep one continuation per iterator, regardless of collection size.
    /// The saved input owns the original collection while downstream code runs.
    fn advance_iteration(&mut self, index: usize) -> Result<(), JqError> {
        let (item, key, len) = match &self.input.value {
            Value::Array(values) => (
                values.get(index).cloned(),
                self.input.path.as_ref().map(|_| Value::int(index as i64)),
                values.len(),
            ),
            Value::Object(values) => (
                values.get_index(index).map(|(_, value)| value.clone()),
                self.input.path.as_ref().and_then(|_| {
                    values.get_index(index).map(|(key, _)| {
                        Value::String(strs::resolve(*key).unwrap_or("").to_string().into())
                    })
                }),
                values.len(),
            ),
            _ => {
                return Err(JqError::Runtime(Value::String(
                    format!(
                        "Cannot iterate over {} ({})",
                        self.input.value.type_name(),
                        value::truncated_repr(&self.input.value)
                    )
                    .into(),
                )));
            }
        };
        if let Some(item) = item {
            if index + 1 < len {
                let mut point = self.snapshot(self.pc);
                point.iteration = Some(index + 1);
                self.choices.push(point);
            }
            self.input.value = item;
            if let (Some(path), Some(key)) = (&mut self.input.path, key) {
                path.push(key);
            }
        } else {
            self.backtrack();
        }
        Ok(())
    }
    fn restore(&mut self, point: ChoicePoint) {
        self.pc = point.pc;
        self.chunk = point.chunk;
        self.calls = point.calls;
        self.input = point.input;
        let previous = std::mem::replace(&mut self.frame, point.frame);
        self.recycle_frame(previous);
        self.operands = point.operands;
        self.path_depth = point.path_depth;
        self.path_depth_stack = point.path_depth_stack;
        self.handlers.restore(point.handlers);
        self.folds = point.folds;
        self.labels = point.labels;
        self.native = point.native;
        self.iteration = point.iteration;
        self.alternatives = point.alternatives;
    }
    fn backtrack(&mut self) {
        while let Some(mut point) = self.choices.pop() {
            if point.skip_if.as_ref().is_some_and(|flag| flag.get()) {
                continue;
            }
            if let Some(crate::jq::builtins::NativeState::Range { next, end, step }) =
                &mut point.native
            {
                let Some(value) = crate::jq::builtins::range_next(next, *end, *step) else {
                    continue;
                };
                // Restore without cloning the large continuation. Re-snapshot only after restore.
                let native = point.native.take();
                self.restore(point);
                let mut continuation = self.snapshot(self.pc);
                continuation.native = native;
                self.choices.push(continuation);
                self.native = None;
                self.input.value = value;
                return;
            }
            self.restore(point);
            return;
        }
        self.done = true;
    }
    pub(crate) fn resume<const LIMITED: bool>(
        &mut self,
        program: &code::Program,
        host: &mut dyn Host,
        budget: &mut usize,
    ) -> VmEvent {
        while !LIMITED || *budget > 0 {
            if self.done {
                return VmEvent::Done;
            }
            if LIMITED {
                *budget -= 1;
            }
            match self.step(program, host) {
                Ok(Some(event)) => return event,
                Ok(None) => {}
                Err(error) => {
                    if let JqError::Runtime(payload) = &error
                        && let Some(handler) = self.handlers.pop()
                    {
                        self.choices.truncate(handler.choices);
                        self.collect.truncate(handler.collections);
                        self.restore(Rc::unwrap_or_clone(handler.recovery));
                        self.input.value = payload.clone();
                        continue;
                    }
                    self.done = true;
                    return VmEvent::Error(error);
                }
            }
        }
        VmEvent::Suspended
    }
    fn step(
        &mut self,
        program: &code::Program,
        host: &mut dyn Host,
    ) -> Result<Option<VmEvent>, JqError> {
        use code::Instruction;
        if let Some(index) = self.iteration.take() {
            self.advance_iteration(index)?;
            return Ok(None);
        }
        if let Some(state) = self.native.take() {
            self.advance_native(state)?;
            return Ok(None);
        }
        let chunk = &program.chunks[self.chunk];
        let instruction = chunk
            .code
            .get(self.pc)
            .ok_or_else(|| JqError::InvalidCode("PC outside chunk".into()))?;
        self.pc += 1;
        match instruction {
            Instruction::Load(value) => {
                self.input = value.clone().into();
            }
            Instruction::SuspendPath => {
                self.path_depth_stack.push(self.path_depth);
                self.path_depth = 0;
            }
            Instruction::ResumePath => {
                self.path_depth = self.path_depth_stack.pop().unwrap_or(0);
            }
            Instruction::BeginPath => {
                self.input.path = Some(Stack::default());
                self.path_depth += 1;
            }
            Instruction::EndPath => {
                self.path_depth = self.path_depth.saturating_sub(1);
                let path = self.input.path.take().ok_or_else(|| {
                    value::error(format!(
                        "Invalid path expression with result {}",
                        &self.input.value.to_compact_json()
                    ))
                })?;
                self.input.value = Value::Array(Rc::new(path.to_vec()));
            }
            Instruction::Read(binding) => {
                let Some(SlotValue::Local(operand)) = self.frame.get(*binding) else {
                    return Err(JqError::Uninitialized);
                };
                self.input = operand.clone();
            }
            Instruction::Drop => {
                let remaining = self
                    .operands
                    .len()
                    .checked_sub(1)
                    .ok_or_else(|| JqError::InvalidCode("operand underflow".into()))?;
                self.operands.truncate(remaining);
            }
            Instruction::Index => {
                let base = self
                    .operands
                    .pop()
                    .ok_or_else(|| JqError::InvalidCode("index input missing".into()))?;
                let Operand {
                    value: base,
                    path: base_path,
                } = base;
                if base_path.is_none() && self.path_depth > 0 {
                    return Err(value::error(format!(
                        "Invalid path expression near attempt to access element {} of {}",
                        &self.input.value.to_compact_json(),
                        &base.to_compact_json()
                    )));
                }
                self.input.path = base_path;
                if let Some(path) = &mut self.input.path {
                    path.push(self.input.value.clone());
                }
                self.input.value = value::index(&base, &self.input.value)?;
            }
            Instruction::Slice => {
                let start = self
                    .operands
                    .pop()
                    .ok_or_else(|| JqError::InvalidCode("slice start missing".into()))?
                    .value;
                let base = self
                    .operands
                    .pop()
                    .ok_or_else(|| JqError::InvalidCode("slice input missing".into()))?;
                let Operand {
                    value: base,
                    path: base_path,
                } = base;
                if base_path.is_none() && self.path_depth > 0 {
                    let mut bounds = indexmap::IndexMap::new();
                    bounds.insert(strs::keyword_start(), start.clone());
                    bounds.insert(strs::keyword_end(), self.input.value.clone());
                    return Err(value::error(format!(
                        "Invalid path expression near attempt to access element {} of {}",
                        &Value::Object(Rc::new(bounds)).to_compact_json(),
                        base.to_compact_json()
                    )));
                }
                self.input.path = base_path;
                if let Some(path) = &mut self.input.path {
                    let mut bounds = indexmap::IndexMap::new();
                    bounds.insert(strs::keyword_start(), start.clone());
                    bounds.insert(strs::keyword_end(), self.input.value.clone());
                    path.push(Value::Object(Rc::new(bounds)));
                }
                self.input.value = value::slice(&base, &start, &self.input.value)?;
            }
            Instruction::Iterate => {
                if self.input.path.is_none() && self.path_depth > 0 {
                    return Err(value::error(format!(
                        "Invalid path expression near attempt to iterate through {}",
                        &self.input.value.to_compact_json()
                    )));
                }
                self.advance_iteration(0)?;
            }

            Instruction::Bind { slot, export } => {
                self.bind(*slot, SlotValue::Local(self.input.clone()));
                if let Some(id) = export {
                    self.exports.insert(*id, self.input.value.clone());
                }
            }
            Instruction::Pop => {
                self.input = self
                    .operands
                    .pop()
                    .ok_or_else(|| JqError::InvalidCode("operand underflow".into()))?;
            }
            Instruction::Define {
                slot,
                function: id,
                captures,
            } => {
                let index = chunk
                    .functions
                    .iter()
                    .position(|f| f.id == *id)
                    .ok_or_else(|| JqError::InvalidCode("missing function".into()))?;
                let function = &chunk.functions[index];
                let mut slots = Frame::empty_slots(function.slots.len());
                for (dst, src) in captures {
                    slots[*dst] = Some(SlotValue::Capture {
                        frame: self.frame.clone(),
                        slot: *src,
                    });
                }
                let closure = Rc::new(Closure {
                    function: Some(index),
                    code: function.code,
                    environment: Rc::new(Frame {
                        base: 0,
                        slots,
                        parent: None,
                    }),
                });
                self.bind(*slot, SlotValue::Filter(closure.clone()));
                if chunk.ir.export_functions.contains(id) {
                    self.exported_functions.insert(*id, closure);
                }
            }
            Instruction::Call(target, args) => {
                let Some(SlotValue::Filter(closure)) = self.frame.get(*target) else {
                    return Err(JqError::Uninitialized);
                };
                let closure = closure.clone();
                let arguments: Vec<_> = args
                    .iter()
                    .map(|offset| {
                        Rc::new(Closure {
                            function: None,
                            code: code::CodeId {
                                chunk: self.chunk,
                                offset: *offset,
                            },
                            environment: self.frame.clone(),
                        })
                    })
                    .collect();
                let frame = if let Some(index) = closure.function {
                    let function = &program.chunks[closure.code.chunk].functions[index];
                    let mut frame =
                        self.new_frame(0, function.params.len() + 1, closure.environment.clone());
                    let slots = &mut Rc::get_mut(&mut frame).unwrap().slots;
                    for (param, argument) in function.params.iter().zip(arguments) {
                        slots[*param] = Some(SlotValue::Filter(argument));
                    }
                    slots[function.self_slot] = Some(SlotValue::Filter(closure.clone()));
                    frame
                } else {
                    closure.environment.clone()
                };
                // A tail call can reuse the return frame; saved choices retain
                // independent caller snapshots for earlier generator branches.
                let mut continuation = self.pc;
                while let Some(Instruction::Jump(target)) = chunk.code.get(continuation) {
                    continuation = *target;
                }
                if matches!(chunk.code.get(continuation), Some(Instruction::Return)) {
                    let previous = std::mem::replace(&mut self.frame, frame);
                    self.recycle_frame(previous);
                } else {
                    self.calls.push(ReturnFrame {
                        code: code::CodeId {
                            chunk: self.chunk,
                            offset: self.pc,
                        },
                        frame: std::mem::replace(&mut self.frame, frame),
                    });
                }
                self.chunk = closure.code.chunk;
                self.pc = closure.code.offset;
            }
            Instruction::Return => {
                let caller = self
                    .calls
                    .pop()
                    .ok_or_else(|| JqError::InvalidCode("return without caller".into()))?;
                let previous = std::mem::replace(&mut self.frame, caller.frame);
                self.recycle_frame(previous);
                self.chunk = caller.code.chunk;
                self.pc = caller.code.offset;
            }
            Instruction::BeginLabel(label) => {
                self.labels.push((
                    *label,
                    Handler {
                        destructure: false,
                        recovery: Rc::new(self.snapshot(self.pc)),
                        choices: self.choices.len(),
                        collections: self.collect.len(),
                    },
                ));
            }
            Instruction::EndLabel => {
                self.labels.truncate(self.labels.len().saturating_sub(1));
            }
            Instruction::Break(label) => {
                let handler = self
                    .labels
                    .iter()
                    .find(|(id, _)| id == label)
                    .map(|(_, handler)| handler.clone())
                    .ok_or_else(|| JqError::InvalidCode("inactive label".into()))?;
                self.choices.truncate(handler.choices);
                self.collect.truncate(handler.collections);
                self.restore(Rc::unwrap_or_clone(handler.recovery));
                self.backtrack();
            }
            Instruction::BeginFold(pc) => {
                self.folds
                    .push(Rc::new(std::cell::RefCell::new(self.input.value.clone())));
                self.choices.push(self.snapshot(*pc));
            }
            Instruction::FoldLoad => {
                let fold = self
                    .folds
                    .last()
                    .ok_or_else(|| JqError::InvalidCode("fold underflow".into()))?;
                self.input.value = fold.replace(Value::Null);
            }
            Instruction::FoldStore => {
                let fold = self
                    .folds
                    .last()
                    .ok_or_else(|| JqError::InvalidCode("fold underflow".into()))?;
                fold.replace(self.input.value.clone());
            }
            Instruction::DropFold => {
                self.folds.pop();
            }
            Instruction::EndFold => {
                let fold = self
                    .folds
                    .pop()
                    .ok_or_else(|| JqError::InvalidCode("fold underflow".into()))?;
                self.input.value = fold.borrow().clone();
            }
            Instruction::BeginTry(pc) | Instruction::BeginDestructure(pc) => {
                self.handlers.push(Handler {
                    destructure: matches!(instruction, Instruction::BeginDestructure(_)),
                    recovery: Rc::new(self.snapshot(*pc)),
                    choices: self.choices.len(),
                    collections: self.collect.len(),
                });
            }
            Instruction::EndTry => {
                // Alternatives created inside this try remain active in their
                // continuation. Close only the lexical try handler.
                self.handlers
                    .remove_first(|h| !h.destructure)
                    .ok_or_else(|| JqError::InvalidCode("missing try handler".into()))?;
            }
            Instruction::BeginAlternative(pc) => {
                let flag = Rc::new(std::cell::Cell::new(false));
                let mut point = self.snapshot(*pc);
                point.skip_if = Some(flag.clone());
                self.choices.push(point);
                self.alternatives.push(flag);
            }
            Instruction::AlternativeItem => {
                let flag = self
                    .alternatives
                    .pop()
                    .ok_or_else(|| JqError::InvalidCode("missing alternative region".into()))?;
                if matches!(self.input.value, Value::Null | Value::Bool(false)) {
                    self.backtrack();
                } else {
                    flag.set(true);
                }
            }
            Instruction::JumpFalse(pc) => {
                if matches!(self.input.value, Value::Null | Value::Bool(false)) {
                    self.pc = *pc;
                }
            }
            Instruction::Push => {
                self.operands.push(self.input.clone());
            }
            Instruction::LoadSaved(depth) => {
                let index = self
                    .operands
                    .len()
                    .checked_sub(depth + 1)
                    .ok_or_else(|| JqError::InvalidCode("operand underflow".into()))?;
                self.input = self.operands[index].clone();
            }
            Instruction::Fork(pc) => self.choices.push(self.snapshot(*pc)),
            Instruction::Jump(pc) => self.pc = *pc,
            Instruction::Backtrack => self.backtrack(),
            Instruction::Yield => return Ok(Some(VmEvent::Output(self.input.value.clone()))),
            Instruction::RunCalc(index) => {
                let program = chunk
                    .calc
                    .get(*index)
                    .ok_or_else(|| JqError::InvalidCode("unknown calc program".into()))?;
                match program.run(&mut self.calc)? {
                    crate::jq::compiler::calc::CalcResult::Value(value) => self.input.value = value,
                    crate::jq::compiler::calc::CalcResult::Empty => self.backtrack(),
                }
            }
            Instruction::BeginCollect(end) => {
                self.choices.push(self.snapshot(*end));
                self.collect.push(Vec::new());
            }
            Instruction::CollectItem => {
                self.collect
                    .last_mut()
                    .ok_or_else(|| JqError::InvalidCode("collect stack underflow".into()))?
                    .push(self.input.value.clone());
            }
            Instruction::EndCollect => {
                let values = self
                    .collect
                    .pop()
                    .ok_or_else(|| JqError::InvalidCode("collect stack underflow".into()))?;
                self.input = Value::Array(Rc::new(values)).into();
            }
            Instruction::MakeObject(pairs) => {
                if self.operands.len() < pairs * 2 + 1 {
                    return Err(JqError::InvalidCode("missing object fields".into()));
                }
                self.input.path = None;
                let fields = self.operands.split_off(self.operands.len() - pairs * 2);
                self.operands
                    .pop()
                    .ok_or_else(|| JqError::InvalidCode("missing call input".into()))?;
                let mut map = indexmap::IndexMap::new();
                for chunk in fields.chunks(2) {
                    let [key, value] = chunk else { unreachable!() };
                    let Value::String(key) = &key.value else {
                        return Err(JqError::Runtime(Value::String(
                            "Object keys must be strings".to_string().into(),
                        )));
                    };
                    map.insert(strs::intern(key), value.value.clone());
                }
                self.input.value = Value::Object(std::rc::Rc::new(map));
            }
            Instruction::BuiltinCall0(instr) => return self.step_builtin0(*instr, host),
            Instruction::BuiltinCall1(instr) => return self.step_builtin1(*instr, host),
            Instruction::BuiltinCall2(instr) => return self.step_builtin2(*instr, host),
            Instruction::BuiltinCall3(instr) => return self.step_builtin3(*instr, host),
        }
        Ok(None)
    }
}
