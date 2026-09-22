//! Conservative facts apply to a single value-argument combination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputUse {
    Ignored,
    Used,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cardinality {
    One,
    ZeroOrOne,
    Many,
    Unknown,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Effects {
    pub may_error: bool,
    pub host_io: bool,
    pub nondeterministic: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Facts {
    pub input: InputUse,
    pub cardinality: Cardinality,
    pub effects: Effects,
}

/// Observations which affect safe stream fusion. These are deliberately
/// separate from the older coarse `Effects` fields so adding a new optimizer
/// does not change the builtin registry ABI.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Observations {
    pub may_empty: bool,
    pub may_stderr: bool,
    pub may_add_input: bool,
    pub may_halt: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub facts: Facts,
    pub observations: Observations,
    pub finite: bool,
}
impl Default for Facts {
    fn default() -> Self {
        Self {
            input: InputUse::Unknown,
            cardinality: Cardinality::Unknown,
            effects: Effects {
                may_error: true,
                host_io: true,
                nondeterministic: true,
            },
        }
    }
}
#[must_use]
pub fn analyze(ir: &super::super::ir::Ir, entry: super::super::ir::ExprId) -> Facts {
    analyze_summary(ir, entry).facts
}

/// Conservative summary used by future fusion passes. Unknown calls and
/// recursive functions remain non-finite and effectful, so failure to prove a
/// property only disables an optimization.
#[must_use]
pub fn analyze_summary(ir: &super::super::ir::Ir, entry: super::super::ir::ExprId) -> Summary {
    let mut active = std::collections::HashSet::new();
    summarize(ir, entry, &mut active)
}

fn summarize(
    ir: &super::super::ir::Ir,
    id: super::super::ir::ExprId,
    active: &mut std::collections::HashSet<super::super::ir::FunctionId>,
) -> Summary {
    use super::super::ir::{CallTarget, Expr, Literal};
    let unknown = || Summary {
        facts: Facts::default(),
        finite: false,
        ..Summary::default()
    };
    match &ir.nodes[id.0].expr {
        Expr::Literal(Literal::Value(_) | Literal::Number(_)) => Summary {
            facts: Facts {
                input: InputUse::Ignored,
                cardinality: Cardinality::One,
                effects: Effects {
                    may_error: false,
                    host_io: false,
                    nondeterministic: false,
                },
            },
            finite: true,
            ..Summary::default()
        },
        Expr::Input | Expr::Read(_) => Summary {
            facts: Facts {
                input: InputUse::Used,
                cardinality: Cardinality::One,
                effects: Effects {
                    may_error: false,
                    host_io: false,
                    nondeterministic: false,
                },
            },
            finite: true,
            ..Summary::default()
        },
        Expr::BuiltinCall { builtin, args } => {
            let mut result = Summary {
                facts: crate::jq::builtins::spec(*builtin).facts,
                finite: true,
                ..Summary::default()
            };
            for arg in args {
                result = combine(result, summarize(ir, *arg, active));
            }
            let name = crate::jq::builtins::spec(*builtin).name;
            let effects = crate::jq::builtins::spec(*builtin).instr.effects();
            result.observations.may_empty |= effects.may_empty;
            result.observations.may_stderr |= effects.may_stderr;
            result.observations.may_add_input |= effects.may_add_input;
            result.observations.may_halt |= effects.may_halt;
            result.finite &= name != "inputs";
            result
        }
        Expr::Call {
            target: CallTarget::Function(function),
            args,
        } => {
            let mut result = Summary::default();
            for arg in args {
                result = combine(result, summarize(ir, *arg, active));
            }
            if !active.insert(*function) {
                return combine(result, unknown());
            }
            let body = ir
                .functions
                .iter()
                .find(|def| def.id == *function)
                .map(|def| summarize(ir, def.body, active));
            active.remove(function);
            combine(result, body.unwrap_or_else(unknown))
        }
        Expr::Call {
            target: CallTarget::FilterParameter(_),
            ..
        } => unknown(),
        Expr::Paths(body) => {
            let mut s = summarize(ir, *body, active);
            s.facts.cardinality = Cardinality::Many;
            s.finite = false;
            s
        }
        Expr::Pipe(items) => items.iter().fold(Summary::default(), |a, id| {
            combine(a, summarize(ir, *id, active))
        }),
        Expr::Concat(items) => {
            let mut result = Summary {
                facts: Facts {
                    cardinality: Cardinality::Many,
                    ..Facts::default()
                },
                finite: true,
                ..Summary::default()
            };
            for id in items {
                result = combine(result, summarize(ir, *id, active));
            }
            result
        }
        Expr::Array(body) => {
            let mut s = summarize(ir, *body, active);
            s.facts.cardinality = Cardinality::One;
            s
        }
        Expr::Object(fields) => fields.iter().fold(
            Summary {
                facts: Facts {
                    input: InputUse::Ignored,
                    cardinality: Cardinality::One,
                    effects: Effects {
                        may_error: false,
                        host_io: false,
                        nondeterministic: false,
                    },
                },
                finite: true,
                ..Summary::default()
            },
            |a, (k, v)| {
                combine(
                    combine(a, summarize(ir, *k, active)),
                    summarize(ir, *v, active),
                )
            },
        ),
        Expr::Bind { source, body, .. } => {
            combine(summarize(ir, *source, active), summarize(ir, *body, active))
        }
        Expr::If { condition, yes, no } => combine(
            summarize(ir, *condition, active),
            combine(summarize(ir, *yes, active), summarize(ir, *no, active)),
        ),
        Expr::Alternative { lhs, rhs } => {
            let mut s = combine(summarize(ir, *lhs, active), summarize(ir, *rhs, active));
            s.observations.may_empty = true;
            s
        }
        Expr::Try { body, handler } => combine(
            summarize(ir, *body, active),
            summarize(ir, *handler, active),
        ),
        Expr::Reduce {
            source,
            init,
            update,
            ..
        } => combine(
            summarize(ir, *source, active),
            combine(summarize(ir, *init, active), summarize(ir, *update, active)),
        ),
        Expr::Foreach {
            source,
            init,
            update,
            extract,
            ..
        } => combine(
            summarize(ir, *source, active),
            combine(
                summarize(ir, *init, active),
                combine(
                    summarize(ir, *update, active),
                    summarize(ir, *extract, active),
                ),
            ),
        ),
        Expr::Scope { body, .. } | Expr::Label { body, .. } => summarize(ir, *body, active),
        Expr::Break(_) => {
            let mut s = unknown();
            s.observations.may_halt = true;
            s
        }
        Expr::Path { base, steps } => {
            let mut s = summarize(ir, *base, active);
            for step in steps {
                match step {
                    super::super::ir::PathStep::Index(i) => {
                        s = combine(s, summarize(ir, *i, active));
                    }
                    super::super::ir::PathStep::Slice { start, end } => {
                        if let Some(i) = start {
                            s = combine(s, summarize(ir, *i, active));
                        }
                        if let Some(i) = end {
                            s = combine(s, summarize(ir, *i, active));
                        }
                    }
                    super::super::ir::PathStep::Iterate => {}
                }
            }
            s
        }
    }
}

const fn combine(mut a: Summary, b: Summary) -> Summary {
    a.facts.input = match (a.facts.input, b.facts.input) {
        (InputUse::Unknown, _) | (_, InputUse::Unknown) => InputUse::Unknown,
        (InputUse::Used, _) | (_, InputUse::Used) => InputUse::Used,
        _ => InputUse::Ignored,
    };
    a.facts.effects.may_error |= b.facts.effects.may_error;
    a.facts.effects.host_io |= b.facts.effects.host_io;
    a.facts.effects.nondeterministic |= b.facts.effects.nondeterministic;
    a.observations.may_empty |= b.observations.may_empty;
    a.observations.may_stderr |= b.observations.may_stderr;
    a.observations.may_add_input |= b.observations.may_add_input;
    a.observations.may_halt |= b.observations.may_halt;
    a.finite &= b.finite;
    a
}
