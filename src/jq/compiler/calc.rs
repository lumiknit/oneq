//! New calc representation. No dependency on legacy `data::calc`.
use crate::{
    data::Value,
    jq::{ir::BuiltinId, vm::JqError},
};

#[derive(Clone, Debug)]
pub enum Instruction {
    Constant {
        dst: usize,
        value: usize,
    },
    Scalar {
        dst: usize,
        builtin: BuiltinId,
        input: usize,
        args: Vec<usize>,
    },
    Return(usize),
}
#[derive(Clone, Debug, Default)]
pub struct CalcProgram {
    pub code: Vec<Instruction>,
    pub constants: Vec<Value>,
    pub registers: usize,
}
#[derive(Debug, Default)]
pub struct CalcContext {
    pub registers: Vec<Value>,
}
pub enum CalcResult {
    Value(Value),
    Empty,
}
impl CalcProgram {
    pub fn run(&self, _context: &mut CalcContext) -> Result<CalcResult, JqError> {
        unimplemented!("M4: execute immutable scalar code in execution-owned scratch")
    }
}
