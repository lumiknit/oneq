use super::code::CodeId;
use super::stack::Stack;
use crate::data::Value;
use std::{cell::Cell, rc::Rc};

#[derive(Clone, Debug)]
pub struct Closure {
    /// Function table index in `code.chunk`; filter arguments have no activation.
    pub function: Option<usize>,
    pub code: CodeId,
    pub environment: Rc<Frame>,
}
#[derive(Debug)]
pub enum SlotValue {
    Local(Operand),
    Filter(Rc<Closure>),
    /// Immutable reference to a slot in a captured lexical environment.
    Capture {
        frame: Rc<Frame>,
        slot: usize,
    },
}

/// A value and its optional source path, shared by bindings and VM continuations.
#[derive(Clone, Debug, Default)]
pub struct Operand {
    pub value: Value,
    pub(crate) path: Option<Stack<Value>>,
}

impl From<Value> for Operand {
    fn from(value: Value) -> Self {
        Self { value, path: None }
    }
}

/// An immutable binding region. Calls and binding occurrences allocate their
/// own region; closures, continuations and choices share all preceding regions.
/// Shared frames are never mutated, including during backtracking. The VM may
/// clear and reuse a region once no continuation or closure retains it.
#[derive(Debug, Default)]
pub struct Frame {
    /// First bytecode slot owned by this binding region.
    pub base: usize,
    pub slots: Vec<Option<SlotValue>>,
    pub parent: Option<Rc<Self>>,
}
impl Frame {
    pub fn get(&self, mut slot: usize) -> Option<&SlotValue> {
        let mut frame = self;
        loop {
            match slot
                .checked_sub(frame.base)
                .and_then(|index| frame.slots.get(index))
                .and_then(Option::as_ref)
            {
                Some(SlotValue::Capture {
                    frame: captured,
                    slot: index,
                }) => {
                    frame = captured;
                    slot = *index;
                }
                Some(value) => return Some(value),
                None => frame = frame.parent.as_deref()?,
            }
        }
    }
    #[must_use]
    pub fn empty_slots(len: usize) -> Vec<Option<SlotValue>> {
        std::iter::repeat_with(|| None).take(len).collect()
    }
    #[must_use]
    pub fn bind(parent: Rc<Self>, slot: usize, value: SlotValue) -> Rc<Self> {
        Rc::new(Self {
            base: slot,
            slots: vec![Some(value)],
            parent: Some(parent),
        })
    }
}
#[derive(Clone, Debug)]
pub(crate) struct ReturnFrame {
    pub code: CodeId,
    pub frame: Rc<Frame>,
}
#[derive(Clone, Debug)]
pub(crate) struct Handler {
    pub destructure: bool,
    pub recovery: Rc<ChoicePoint>,
    pub choices: usize,
    pub collections: usize,
}
#[derive(Clone, Debug)]
pub(crate) struct ChoicePoint {
    pub pc: usize,
    pub chunk: usize,
    pub input: Operand,
    pub frame: Rc<Frame>,
    pub operands: Stack<Operand>,
    pub path_depth: usize,
    pub path_depth_stack: Stack<usize>,
    pub calls: Stack<ReturnFrame>,
    pub handlers: Stack<Handler, 1>,
    pub alternatives: Stack<Rc<Cell<bool>>>,
    pub skip_if: Option<Rc<Cell<bool>>>,
    pub folds: Stack<Rc<std::cell::RefCell<Value>>>,
    pub labels: Stack<(crate::jq::ir::LabelId, Handler), 1>,
    pub iteration: Option<usize>,
    pub native: Option<crate::jq::builtins::NativeState>,
}
