//! The outer input loop and input/inputs share exactly this cursor.
use super::JqError;
use crate::{data::Value, io::SharedInputTracker, strs::Symbol};

pub trait Host {
    fn next_input(&mut self) -> Option<Result<Value, JqError>>;
    fn environment(&mut self) -> Result<Value, JqError> {
        Err(JqError::Unsupported(
            "env is not provided by this host".into(),
        ))
    }
    /// Filename of whatever input is currently being read, if the host tracks one.
    fn input_filename(&self) -> Option<Symbol> {
        None
    }
    /// Line number within the current input, if the host tracks one.
    fn input_line_number(&self) -> Option<usize> {
        None
    }
    /// jq's `modulemeta`: given a module name as input, resolve and parse
    /// that module (using the same search path as `import`/`include`) and
    /// return its computed metadata (`{..declared fields.., deps, defs}`).
    fn modulemeta(&mut self, _name: &Value) -> Result<Value, JqError> {
        Err(JqError::Unsupported(
            "modulemeta is not provided by this host".into(),
        ))
    }
}

pub struct InputHost<I> {
    inputs: I,
    tracker: Option<SharedInputTracker>,
}
impl<I> InputHost<I> {
    pub fn new(inputs: I) -> Self {
        Self {
            inputs,
            tracker: None,
        }
    }

    pub fn with_tracker(inputs: I, tracker: SharedInputTracker) -> Self {
        Self {
            inputs,
            tracker: Some(tracker),
        }
    }
}
impl<I: Iterator<Item = Result<Value, JqError>>> Host for InputHost<I> {
    fn next_input(&mut self) -> Option<Result<Value, JqError>> {
        self.inputs.next()
    }
    fn input_filename(&self) -> Option<Symbol> {
        self.tracker.as_ref().and_then(|t| t.borrow().filename)
    }
    fn input_line_number(&self) -> Option<usize> {
        self.tracker.as_ref().map(|t| t.borrow().line_number)
    }
}
