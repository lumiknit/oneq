//! Direct builtin dispatch. Every arm has a statically known target;
//! runtime execution never consults BuiltinSpec or calls through a function pointer.
use super::{JqError, Vm, VmEvent, host::Host, value};
use crate::{
    data::Value,
    jq::builtins::{self, BuiltinInstr},
    strs,
};

macro_rules! dispatch {
    ($($variant:ident($arity:literal) => $module:ident::$function:ident,)*) => {
        impl Vm {
            // This is an instruction-dispatch fragment with one call site,
            // not another runtime call boundary. Large builtin bodies remain
            // ordinary direct calls, with their own inlining decisions.
            #[inline(always)]
            pub(super) fn step_builtin<const PATH: bool>(
                &mut self, instr: BuiltinInstr, host: &mut dyn Host,
            ) -> Result<Option<VmEvent>, JqError> {
                let arity = instr.arity();
                let remaining = self.operands.len().checked_sub(arity + 1)
                    .ok_or_else(|| JqError::InvalidCode("missing builtin operands".into()))?;
                if PATH {
                    self.operand_paths.truncate(remaining);
                }
                self.path = None;
                let mut storage = [const { Value::Null }; BuiltinInstr::MAX_ARITY];
                let args = &mut storage[..arity];
                for arg in args.iter_mut() {
                    *arg = self.operands.pop().unwrap();
                }
                // Share operand extraction across arms to keep the hot dispatch
                // compact. Infix expressions push in the opposite order.
                if !instr.is_infix_operator() {
                    args.reverse();
                }
                let input = self.operands.pop().unwrap();
                match instr {
                    $(BuiltinInstr::$variant => {
                        self.input = builtins::$module::$function(&input, args)?;
                        // Only getpath has this path-producing scalar behavior.
                        // The instruction check folds away in every other arm.
                        if PATH && matches!(BuiltinInstr::$variant, BuiltinInstr::GetPath)
                            && self.path_depth > 0
                            && let Some(Value::Array(steps)) = args.first()
                        {
                            self.path = Some(steps.iter().cloned().collect());
                        }
                    },)*
                    BuiltinInstr::Add => {
                        // Release evaluation-only aliases before COW. Captured
                        // bindings and choices must still retain their values.
                        self.input = Value::Null;
                        drop(input);
                        self.input = builtins::scalar::add_owned(args)?;
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
                        self.input = host.next_input().unwrap_or_else(||
                            Err(JqError::Runtime(Value::String("break".to_string().into()))))?;
                    }
                    BuiltinInstr::Env => {
                        self.input = host.environment()?;
                    }
                    BuiltinInstr::InputFilename => {
                        self.input = match host.input_filename().and_then(strs::resolve) {
                            Some(name) => Value::String(name.to_string().into()),
                            None => Value::Null,
                        };
                    }
                    BuiltinInstr::InputLineNumber => {
                        self.input = Value::Float(host.input_line_number().unwrap_or(0) as f64);
                    }
                    BuiltinInstr::ModuleMeta => {
                        self.input = host.modulemeta(&input)?;
                    }
                    BuiltinInstr::HaveDecnum | BuiltinInstr::HaveLiteralNumbers => {
                        self.input = Value::Bool(matches!(instr, BuiltinInstr::HaveDecnum));
                    }
                }
                Ok(None)
            }
        }
    };
}
builtins::scalar_instructions!(dispatch);
