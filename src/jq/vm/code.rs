//! Chunks become immutable when appended. IDs remain valid across later appends.
use crate::data::Value;
use crate::jq::{compiler::calc::CalcProgram, ir::*};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodeId {
    pub(crate) chunk: usize,
    pub(crate) offset: usize,
}

#[derive(Clone, Debug)]
pub enum Instruction {
    Load(Value),
    Read(usize),
    Push,
    Pop,
    Drop,
    Index,
    BeginPath,
    EndPath,
    Iterate,
    Slice,
    /// Temporarily treats path tracking as "not required" (as if outside any
    /// `path(...)`/assignment) for a nested value-position sub-expression -
    /// e.g. a dynamic index key `.[expr]` - even while still lexically
    /// inside an outer `path(...)`. Without this, something like
    /// `path(.a[path(.b)[0]])` would wrongly raise "Invalid path expression"
    /// for the `[0]` that merely extracts a plain value out of the inner
    /// `path(.b)`'s already-materialized result array.
    SuspendPath,
    /// Restores the path-required depth saved by the matching `SuspendPath`.
    ResumePath,
    Bind {
        slot: usize,
        export: Option<BindingId>,
    },
    Define {
        slot: usize,
        function: FunctionId,
        captures: Vec<(usize, usize)>,
    },
    Call(usize, Vec<usize>),
    Return,
    JumpFalse(usize),
    /// Retry the next pattern on errors anywhere in the continuation, including
    /// after returning to a caller. Backtracking restores the previous handlers.
    BeginDestructure(usize),
    BeginTry(usize),
    EndTry,
    BeginAlternative(usize),
    AlternativeItem,
    BeginFold(usize),
    FoldLoad,
    FoldStore,
    EndFold,
    DropFold,
    BeginLabel(LabelId),
    EndLabel,
    Break(LabelId),
    LoadSaved(usize),
    Fork(usize),
    Jump(usize),
    BuiltinCall0(crate::jq::builtins::BuiltinOp0),
    BuiltinCall1(crate::jq::builtins::BuiltinOp1),
    BuiltinCall2(crate::jq::builtins::BuiltinOp2),
    BuiltinCall3(crate::jq::builtins::BuiltinOp3),
    RunCalc(usize),
    /// Opens a collection region: pushes a resume choice at `end` and starts a
    /// fresh accumulator. Falls through into the collected sub-expression.
    BeginCollect(usize),
    /// Appends the current input to the innermost accumulator. Callers must
    /// follow with an explicit Backtrack to search for further items.
    CollectItem,
    /// Placed at a BeginCollect's `end`: pops the accumulator into an array
    /// and continues with it as input. The backtrack that reaches this point
    /// already restored the pre-collection input/frame/operands.
    EndCollect,
    /// Pops `2 * n` values (key, value pairs, in source order) plus the
    /// original input, and continues with the built object as input.
    MakeObject(usize),
    Yield,
    Backtrack,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub id: FunctionId,
    pub code: CodeId,
    pub params: Vec<usize>,
    pub slots: Vec<SlotKey>,
    pub captures: Vec<usize>,
    pub self_slot: usize,
}

#[derive(Clone, Debug)]
pub struct Chunk {
    /// Dense entry-frame slots, independent of session-wide IR identities.
    pub slots: Vec<SlotKey>,
    /// Immutable JSON imports with stable lexical declaration identities.
    pub data_bindings: Vec<(BindingId, Value)>,
    pub definition_only: bool,
    pub ir: Ir,
    pub entry: ExprId,
    pub code: Vec<Instruction>,
    pub functions: Vec<Function>,
    pub calc: Vec<CalcProgram>,
}

#[derive(Debug, Default)]
pub(crate) struct Program {
    pub chunks: Vec<Chunk>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SlotKey {
    Local(BindingId),
    Filter(BindingId),
    Function(FunctionId),
}
