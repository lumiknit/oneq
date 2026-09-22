//! Direct builtin dispatch. Every arm has a statically known target;
//! runtime execution never consults `BuiltinSpec` or calls through a function pointer.
use super::{JqError, Vm, VmEvent, host::Host, value};
use crate::{
    data::Value,
    jq::builtins::{self, BuiltinInstr, BuiltinOp0, BuiltinOp1, BuiltinOp2, BuiltinOp3},
    strs,
};

macro_rules! dispatch {
    ($($variant:ident($arity:literal) => $module:ident::$function:ident,)*) => {
        impl Vm {
            // This is an instruction-dispatch fragment with one call site,
            // not another runtime call boundary. Large builtin bodies remain
            // ordinary direct calls, with their own inlining decisions.
            #[inline]
            pub(super) fn step_builtin<const N: usize>(
                &mut self, instr: BuiltinInstr, host: &mut dyn Host,
            ) -> Result<Option<VmEvent>, JqError> {
                self.input.path = None;
                debug_assert_eq!(instr.arity(), N);
                let mut storage = [const { Value::Null }; N];
                let args = &mut storage;
                let input = if N == 0 {
                    self.input.value.clone()
                } else {
                    for arg in args.iter_mut() { *arg = self.operands.pop().unwrap().value; }
                    if !instr.is_infix_operator() { args.reverse(); }
                    self.operands.pop().unwrap().value
                };
                self.apply_builtin(instr, input, args, host)
            }

            #[inline]
            fn apply_builtin<const N: usize>(
                &mut self, instr: BuiltinInstr, input: Value, args: &mut [Value; N],
                host: &mut dyn Host,
            ) -> Result<Option<VmEvent>, JqError> {
                match instr {
                    $(BuiltinInstr::$variant => {
                        self.input.value = builtins::$module::$function(&input, args)?;
                        // Only getpath has this path-producing scalar behavior.
                        // The instruction check folds away in every other arm.
                        if matches!(BuiltinInstr::$variant, BuiltinInstr::GetPath)
                            && self.path_depth > 0
                            && let Some(Value::Array(steps)) = args.first()
                        {
                            self.input.path = Some(steps.iter().cloned().collect());
                        }
                    },)*
                    BuiltinInstr::Add => {
                        // Release evaluation-only aliases before COW. Captured
                        // bindings and choices must still retain their values.
                        self.input.value = Value::Null;
                        drop(input);
                        self.input.value = builtins::scalar::add_owned(args)?;
                    }
                    BuiltinInstr::Range => {
                        let mut state = builtins::range();
                        state.initialize(&input, args)?;
                        self.advance_native(state)?;
                    }
                    BuiltinInstr::Path => {
                        return Err(JqError::InvalidCode("uncompiled path filter".into()));
                    }
                    BuiltinInstr::Empty => {
                        self.backtrack();
                    }
                    BuiltinInstr::Error => {
                        return Err(JqError::Runtime(input));
                    }
                    BuiltinInstr::HaltError => {
                        let code = args[0].as_number().ok_or_else(||
                            value::error("halt_error requires a numeric exit code"))? as i32;
                        return Ok(Some(VmEvent::Halt { code, value: input }));
                    }
                    BuiltinInstr::Halt => {
                        return Ok(Some(VmEvent::Halt { code: 0, value: Value::Null }));
                    }
                    BuiltinInstr::Input => {
                        self.input.value = host.next_input().unwrap_or_else(||
                            Err(JqError::Runtime(Value::String("break".to_string().into()))))?;
                    }
                    BuiltinInstr::Env => {
                        self.input.value = host.environment()?;
                    }
                    BuiltinInstr::InputFilename => {
                        self.input.value = match host.input_filename().and_then(strs::resolve) {
                            Some(name) => Value::String(name.to_string().into()),
                            None => Value::Null,
                        };
                    }
                    BuiltinInstr::InputLineNumber => {
                        self.input.value = Value::Float(host.input_line_number().unwrap_or(0) as f64);
                    }
                    BuiltinInstr::ModuleMeta => {
                        self.input.value = host.modulemeta(&input)?;
                    }
                    BuiltinInstr::HaveDecnum | BuiltinInstr::HaveLiteralNumbers => {
                        self.input.value = Value::Bool(matches!(instr, BuiltinInstr::HaveDecnum));
                    }
                }
                Ok(None)
            }
        }
    };
}

builtins::scalar_instructions!(dispatch);

impl Vm {
    #[inline]
    pub(super) fn step_infix_const(
        &mut self,
        op: BuiltinOp2,
        right: &Value,
        host: &mut dyn Host,
    ) -> Result<Option<VmEvent>, JqError> {
        debug_assert!(op.0.is_infix_operator());
        self.input.path = None;
        let mut args = [std::mem::take(&mut self.input.value), right.clone()];
        self.apply_builtin(op.0, Value::Null, &mut args, host)
    }

    #[inline]
    pub(super) fn step_builtin0(
        &mut self,
        op: BuiltinOp0,
        host: &mut dyn Host,
    ) -> Result<Option<VmEvent>, JqError> {
        self.step_builtin::<0>(op.0, host)
    }
    #[inline]
    pub(super) fn step_builtin1(
        &mut self,
        op: BuiltinOp1,
        host: &mut dyn Host,
    ) -> Result<Option<VmEvent>, JqError> {
        self.step_builtin::<1>(op.0, host)
    }
    #[inline]
    pub(super) fn step_builtin2(
        &mut self,
        op: BuiltinOp2,
        host: &mut dyn Host,
    ) -> Result<Option<VmEvent>, JqError> {
        self.step_builtin::<2>(op.0, host)
    }
    #[inline]
    pub(super) fn step_builtin3(
        &mut self,
        op: BuiltinOp3,
        host: &mut dyn Host,
    ) -> Result<Option<VmEvent>, JqError> {
        self.step_builtin::<3>(op.0, host)
    }
}
