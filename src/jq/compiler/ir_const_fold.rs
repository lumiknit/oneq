//! Small, bounded folds using the same scalar semantics as the VM.
use super::rewrite::{self, Item};
use crate::{
    data::{self, Value},
    jq::{
        builtins::{self, BuiltinInstr, scalar, strings},
        ir::{Expr, ExprId, Ir, Literal, PathStep},
    },
    strs,
};

fn literal(ir: &Ir, id: ExprId) -> Option<Value> {
    match &ir.nodes[id.0].expr {
        Expr::Literal(Literal::Value(value)) => Some(value.clone()),
        Expr::Literal(Literal::Number(raw)) => data::parse_json_str(raw).ok(),
        _ => None,
    }
}

pub(super) fn optimize(ir: &mut Ir, entry: &mut ExprId) {
    for item in rewrite::postorder(ir, *entry) {
        let Item::Expr(id) = item else { continue };
        let replacement = match ir.nodes[id.0].expr.clone() {
            Expr::BuiltinCall { builtin, args } => {
                let instr = builtins::spec(builtin).instr;
                let values: Option<Vec<_>> = args.iter().map(|id| literal(ir, *id)).collect();
                values.and_then(|mut args| {
                    // String repetition, regexes and other potentially expensive
                    // operations deliberately stay out of this small pass.
                    let result = match instr {
                        BuiltinInstr::Add => scalar::add_owned(&mut args),
                        BuiltinInstr::Sub => scalar::subtract(&Value::Null, &args),
                        BuiltinInstr::Mul if args.iter().all(|v| v.as_number().is_some()) => {
                            scalar::multiply(&Value::Null, &args)
                        }
                        BuiltinInstr::Div if args.iter().all(|v| v.as_number().is_some()) => {
                            scalar::divide(&Value::Null, &args)
                        }
                        BuiltinInstr::Rem => scalar::modulo(&Value::Null, &args),
                        BuiltinInstr::Neg => scalar::negate(&Value::Null, &args),
                        BuiltinInstr::Eq => scalar::equal(&Value::Null, &args),
                        BuiltinInstr::Ne => scalar::unequal(&Value::Null, &args),
                        BuiltinInstr::Lt => scalar::less(&Value::Null, &args),
                        BuiltinInstr::Le => scalar::less_equal(&Value::Null, &args),
                        BuiltinInstr::Gt => scalar::greater(&Value::Null, &args),
                        BuiltinInstr::Ge => scalar::greater_equal(&Value::Null, &args),
                        _ => return None,
                    };
                    // Keep failures at runtime, including in try/catch and
                    // branches which may never be evaluated.
                    result
                        .ok()
                        .map(|value| Expr::Literal(Literal::Value(value)))
                })
            }
            Expr::Pipe(items) => {
                let mut folded = Vec::new();
                for item in items {
                    let value = folded.last().and_then(|id| literal(ir, *id));
                    let result = match (&ir.nodes[item.0].expr, value) {
                        (Expr::BuiltinCall { builtin, args }, Some(input)) if args.is_empty() => {
                            match builtins::spec(*builtin).instr {
                                BuiltinInstr::Trim => strings::trim(&input, &[]).ok(),
                                BuiltinInstr::Ltrim => strings::ltrim(&input, &[]).ok(),
                                BuiltinInstr::Rtrim => strings::rtrim(&input, &[]).ok(),
                                BuiltinInstr::AsciiDowncase => {
                                    strings::ascii_downcase(&input, &[]).ok()
                                }
                                BuiltinInstr::AsciiUpcase => {
                                    strings::ascii_upcase(&input, &[]).ok()
                                }
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    if let Some(value) = result {
                        folded.pop();
                        folded.push(ir.push(
                            Expr::Literal(Literal::Value(value)),
                            ir.nodes[item.0].span.clone(),
                        ));
                    } else {
                        folded.push(item);
                    }
                }
                Some(if folded.len() == 1 {
                    ir.nodes[folded[0].0].expr.clone()
                } else {
                    Expr::Pipe(folded)
                })
            }
            Expr::Array(_) | Expr::Object(_) => {
                use crate::jq::vm::code::{BuildValue, ContainerPlan};
                super::containers::plan(ir, id).and_then(|(plan, dynamic)| {
                    if !dynamic.is_empty() {
                        return None;
                    }
                    let constant = |field| match field {
                        BuildValue::Constant(value) => value,
                        BuildValue::Dynamic => unreachable!(),
                    };
                    let value = match plan {
                        ContainerPlan::Array(fields) => Value::Array(std::rc::Rc::new(
                            fields.into_iter().map(constant).collect(),
                        )),
                        ContainerPlan::Object(fields) => Value::Object(std::rc::Rc::new(
                            fields
                                .into_iter()
                                .map(|(key, value)| (key, constant(value)))
                                .collect(),
                        )),
                    };
                    Some(Expr::Literal(Literal::Value(value)))
                })
            }
            Expr::Path {
                mut base,
                mut steps,
            } => {
                for step in &mut steps {
                    if let PathStep::Index(index) = step
                        && let Some(Value::String(key)) = literal(ir, *index)
                    {
                        *step = PathStep::Key(strs::intern(&key));
                    }
                }
                let keys: Option<Vec<_>> = steps
                    .iter()
                    .map(|step| {
                        if let PathStep::Key(key) = step {
                            Some(*key)
                        } else {
                            None
                        }
                    })
                    .collect();
                Some(if let Some(mut steps) = keys {
                    if let Expr::ConstPath {
                        base: parent,
                        steps: prefix,
                    } = &ir.nodes[base.0].expr
                    {
                        base = *parent;
                        let mut combined = prefix.clone();
                        combined.append(&mut steps);
                        steps = combined;
                    }
                    Expr::ConstPath { base, steps }
                } else {
                    Expr::Path { base, steps }
                })
            }
            _ => None,
        };
        if let Some(expr) = replacement {
            ir.nodes[id.0].expr = expr;
        }
    }
    // Removes folded operands and interned string literals, remapping IDs.
    super::templates::compact(ir, entry);
}
