//! One registry for resolution, invocation, analysis and future documentation.
use crate::{
    data::Value,
    jq::{
        compiler::analyze::Facts,
        ir::BuiltinId,
        vm::{JqError, frame::Closure},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamMode {
    Value,
    Filter,
}
mod instr;
pub(crate) use instr::scalar_instructions;
pub use instr::{BuiltinInstr, BuiltinOp0, BuiltinOp1, BuiltinOp2, BuiltinOp3};
#[derive(Clone, Copy, Debug, Default)]
pub struct BuiltinEffects {
    pub may_empty: bool,
    pub may_stderr: bool,
    pub may_add_input: bool,
    pub may_halt: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct CalcRecipe {
    pub instr: BuiltinInstr,
}
#[derive(Clone, Debug)]
pub struct BuiltinSpec {
    pub name: &'static str,
    pub params: &'static [ParamMode],
    pub facts: Facts,
    pub instr: BuiltinInstr,
    pub calc: Option<CalcRecipe>,
    pub docs: &'static str,
}
impl BuiltinSpec {
    /// True for jq's built-in infix operators (`+`, `==`, ...). Unlike
    /// ordinary multi-arg calls - which nest left-to-right, leftmost
    /// varying slowest - jq's own compiler (`gen_binop`) nests these with
    /// the *right* operand outer instead, so `(1,2)+(10,20)` yields
    /// `11,12,21,22` rather than `11,21,12,22`.
    pub fn is_infix_operator(&self) -> bool {
        self.instr.is_infix_operator()
    }
}

pub(crate) mod collections;
pub(crate) mod datetime;
pub(crate) mod math;
pub(crate) mod random;
pub(crate) mod regex;
pub(crate) mod scalar;
pub(crate) mod strings;

const PURE: Facts = Facts {
    input: crate::jq::compiler::analyze::InputUse::Used,
    cardinality: crate::jq::compiler::analyze::Cardinality::One,
    effects: crate::jq::compiler::analyze::Effects {
        may_error: false,
        host_io: false,
        nondeterministic: false,
    },
};

const FALLIBLE: Facts = Facts {
    effects: crate::jq::compiler::analyze::Effects {
        may_error: true,
        ..PURE.effects
    },
    ..PURE
};

const STREAM: Facts = Facts {
    cardinality: crate::jq::compiler::analyze::Cardinality::Many,
    ..PURE
};

const HOST: Facts = Facts {
    input: crate::jq::compiler::analyze::InputUse::Ignored,
    effects: crate::jq::compiler::analyze::Effects {
        may_error: true,
        host_io: true,
        nondeterministic: true,
    },
    ..STREAM
};

/// Like `HOST`, but for a host builtin that actually reads its input value
/// (`modulemeta` takes the module name to look up as its input).
const HOST_WITH_INPUT: Facts = Facts {
    input: crate::jq::compiler::analyze::InputUse::Used,
    ..HOST
};

const RANDOM_FACTS: Facts = Facts {
    effects: crate::jq::compiler::analyze::Effects {
        nondeterministic: true,
        ..PURE.effects
    },
    ..PURE
};

macro_rules! scalar_spec {
    ($name:literal, [$($param:expr),*], $instr:ident) => {
        BuiltinSpec { name: $name, params: &[$($param),*], facts: FALLIBLE,
            instr: BuiltinInstr::$instr,
            calc: Some(CalcRecipe { instr: BuiltinInstr::$instr }), docs: "Scalar jq operation." }
    };
}

pub fn registry() -> &'static [BuiltinSpec] {
    &[
        scalar_spec!("acos", [], Acos),
        scalar_spec!("acosh", [], Acosh),
        scalar_spec!("asin", [], Asin),
        scalar_spec!("asinh", [], Asinh),
        scalar_spec!("atan", [], Atan),
        scalar_spec!("atanh", [], Atanh),
        scalar_spec!("cos", [], Cos),
        scalar_spec!("cosh", [], Cosh),
        scalar_spec!("sin", [], Sin),
        scalar_spec!("sinh", [], Sinh),
        scalar_spec!("tan", [], Tan),
        scalar_spec!("tanh", [], Tanh),
        scalar_spec!("exp", [], Exp),
        scalar_spec!("exp2", [], Exp2),
        scalar_spec!("expm1", [], Expm1),
        scalar_spec!("log", [], Log),
        scalar_spec!("log2", [], Log2),
        scalar_spec!("log10", [], Log10),
        scalar_spec!("log1p", [], Log1p),
        scalar_spec!("sqrt", [], Sqrt),
        scalar_spec!("cbrt", [], Cbrt),
        scalar_spec!("floor", [], Floor),
        scalar_spec!("ceil", [], Ceil),
        scalar_spec!("trunc", [], Trunc),
        scalar_spec!("round", [], Round),
        scalar_spec!("fabs", [], Fabs),
        scalar_spec!("erf", [], Erf),
        scalar_spec!("erfc", [], Erfc),
        scalar_spec!("lgamma", [], Lgamma),
        scalar_spec!("tgamma", [], Tgamma),
        scalar_spec!("j0", [], J0),
        scalar_spec!("j1", [], J1),
        scalar_spec!("y0", [], Y0),
        scalar_spec!("y1", [], Y1),
        scalar_spec!("exp10", [], Exp10),
        scalar_spec!("significand", [], Significand),
        scalar_spec!("logb", [], Logb),
        scalar_spec!("nearbyint", [], Nearbyint),
        scalar_spec!("rint", [], Rint),
        scalar_spec!("frexp", [], Frexp),
        scalar_spec!("modf", [], Modf),
        scalar_spec!("isnan", [], Isnan),
        scalar_spec!("isinfinite", [], Isinfinite),
        scalar_spec!("isnormal", [], Isnormal),
        scalar_spec!("atan2", [ParamMode::Value, ParamMode::Value], Atan2),
        scalar_spec!("pow", [ParamMode::Value, ParamMode::Value], Pow),
        scalar_spec!("hypot", [ParamMode::Value, ParamMode::Value], Hypot),
        scalar_spec!("fmod", [ParamMode::Value, ParamMode::Value], Fmod),
        scalar_spec!("remainder", [ParamMode::Value, ParamMode::Value], Remainder),
        scalar_spec!("copysign", [ParamMode::Value, ParamMode::Value], Copysign),
        scalar_spec!("nextafter", [ParamMode::Value, ParamMode::Value], Nextafter),
        scalar_spec!("fdim", [ParamMode::Value, ParamMode::Value], Fdim),
        scalar_spec!("fmax", [ParamMode::Value, ParamMode::Value], Fmax),
        scalar_spec!("fmin", [ParamMode::Value, ParamMode::Value], Fmin),
        scalar_spec!("ldexp", [ParamMode::Value, ParamMode::Value], Ldexp),
        scalar_spec!("jn", [ParamMode::Value, ParamMode::Value], Jn),
        scalar_spec!("yn", [ParamMode::Value, ParamMode::Value], Yn),
        scalar_spec!(
            "fma",
            [ParamMode::Value, ParamMode::Value, ParamMode::Value],
            Fma
        ),
        scalar_spec!("nan", [], Nan),
        scalar_spec!("gamma", [], Lgamma),
        scalar_spec!("lgamma_r", [], LgammaR),
        scalar_spec!("ilogb", [], Ilogb),
        scalar_spec!("drem", [ParamMode::Value, ParamMode::Value], Remainder),
        scalar_spec!("scalb", [ParamMode::Value, ParamMode::Value], Scalb),
        scalar_spec!("scalbln", [ParamMode::Value, ParamMode::Value], Scalb),
        scalar_spec!("infinite", [], Infinite),
        scalar_spec!("builtins", [], Builtins),
        BuiltinSpec {
            name: "random",
            params: &[],
            facts: RANDOM_FACTS,
            instr: BuiltinInstr::Random,
            calc: None,
            docs: "Return a random real number in [0, 1).",
        },
        BuiltinSpec {
            name: "randint",
            params: &[ParamMode::Value, ParamMode::Value],
            facts: RANDOM_FACTS,
            instr: BuiltinInstr::Randint2,
            calc: None,
            docs: "Return a random integer in [a, b).",
        },
        BuiltinSpec {
            name: "choice",
            params: &[],
            facts: RANDOM_FACTS,
            instr: BuiltinInstr::Choice,
            calc: None,
            docs: "Return one random element from the input array.",
        },
        scalar_spec!(
            "_match_impl",
            [ParamMode::Value, ParamMode::Value, ParamMode::Value],
            MatchImpl
        ),
        scalar_spec!("sort", [], Sort),
        scalar_spec!("unique", [], Unique),
        scalar_spec!("min", [], Min),
        scalar_spec!("max", [], Max),
        scalar_spec!("contains", [ParamMode::Value], Contains),
        scalar_spec!("_sort_by_keys", [], SortByKeys),
        scalar_spec!("_group_sorted", [], GroupSorted),
        scalar_spec!("utf8bytelength", [], Utf8bytelength),
        scalar_spec!("explode", [], Explode),
        scalar_spec!("implode", [], Implode),
        scalar_spec!("startswith", [ParamMode::Value], Startswith),
        scalar_spec!("endswith", [ParamMode::Value], Endswith),
        scalar_spec!("trim", [], Trim),
        scalar_spec!("ltrim", [], Ltrim),
        scalar_spec!("rtrim", [], Rtrim),
        scalar_spec!("split", [ParamMode::Value], Split),
        scalar_spec!("bsearch", [ParamMode::Value], Bsearch),
        scalar_spec!("@text", [], Text),
        scalar_spec!("@json", [], AsJson),
        scalar_spec!("@html", [], Html),
        scalar_spec!("@htmld", [], Htmld),
        scalar_spec!("@uri", [], Uri),
        scalar_spec!("@urid", [], Urid),
        scalar_spec!("@base64", [], Base64),
        scalar_spec!("@base64d", [], Base64d),
        scalar_spec!("@base32", [], Base32),
        scalar_spec!("@base32d", [], Base32d),
        scalar_spec!("@hex", [], Hex),
        scalar_spec!("@hexd", [], Hexd),
        scalar_spec!("ascii", [], Ascii),
        scalar_spec!("@sh", [], Sh),
        scalar_spec!("@csv", [], Csv),
        scalar_spec!("@tsv", [], Tsv),
        scalar_spec!("sha1", [], Sha1),
        scalar_spec!("sha256", [], Sha256),
        scalar_spec!("sha512", [], Sha512),
        BuiltinSpec {
            name: "halt_error",
            params: &[ParamMode::Value],
            facts: FALLIBLE,
            instr: BuiltinInstr::HaltError,
            calc: None,
            docs: "Halt with an exit code and the raw input on stderr.",
        },
        BuiltinSpec {
            name: "path",
            params: &[ParamMode::Filter],
            facts: STREAM,
            instr: BuiltinInstr::Path,
            calc: None,
            docs: "Stream selected paths.",
        },
        scalar_spec!("getpath", [ParamMode::Value], GetPath),
        scalar_spec!("delpaths", [ParamMode::Value], DelPaths),
        scalar_spec!("setpath", [ParamMode::Value, ParamMode::Value], SetPath),
        scalar_spec!("_strindices", [ParamMode::Value], Strindices),
        scalar_spec!("_sort_by_impl", [ParamMode::Value], SortByImpl),
        scalar_spec!("_group_by_impl", [ParamMode::Value], GroupByImpl),
        scalar_spec!("_max_by_impl", [ParamMode::Value], MaxByImpl),
        scalar_spec!("_min_by_impl", [ParamMode::Value], MinByImpl),
        scalar_spec!("_unique_by_impl", [ParamMode::Value], UniqueByImpl),
        BuiltinSpec {
            name: "range",
            params: &[ParamMode::Value, ParamMode::Value],
            facts: STREAM,
            instr: BuiltinInstr::Range,
            calc: None,
            docs: "Stream a numeric interval.",
        },
        BuiltinSpec {
            name: "+",
            params: &[ParamMode::Value, ParamMode::Value],
            facts: FALLIBLE,
            instr: BuiltinInstr::Add,
            calc: Some(CalcRecipe {
                instr: BuiltinInstr::Add,
            }),
            docs: "Scalar jq operation.",
        },
        scalar_spec!("-", [ParamMode::Value, ParamMode::Value], Sub),
        scalar_spec!("-", [ParamMode::Value], Neg),
        scalar_spec!("*", [ParamMode::Value, ParamMode::Value], Mul),
        scalar_spec!("/", [ParamMode::Value, ParamMode::Value], Div),
        scalar_spec!("%", [ParamMode::Value, ParamMode::Value], Rem),
        scalar_spec!("==", [ParamMode::Value, ParamMode::Value], Eq),
        scalar_spec!("!=", [ParamMode::Value, ParamMode::Value], Ne),
        scalar_spec!("<", [ParamMode::Value, ParamMode::Value], Lt),
        scalar_spec!("<=", [ParamMode::Value, ParamMode::Value], Le),
        scalar_spec!(">", [ParamMode::Value, ParamMode::Value], Gt),
        scalar_spec!(">=", [ParamMode::Value, ParamMode::Value], Ge),
        scalar_spec!("not", [], Not),
        scalar_spec!("tostring", [], Tostring),
        scalar_spec!("tojson", [], Tojson),
        scalar_spec!("fromjson", [], Fromjson),
        scalar_spec!("tonumber", [], Tonumber),
        scalar_spec!("keys", [], Keys),
        scalar_spec!("keys_unsorted", [], KeysUnsorted),
        scalar_spec!("has", [ParamMode::Value], Has),
        BuiltinSpec {
            name: "type",
            params: &[],
            facts: PURE,
            instr: BuiltinInstr::Type,
            calc: Some(CalcRecipe {
                instr: BuiltinInstr::Type,
            }),
            docs: "Return the input type.",
        },
        BuiltinSpec {
            name: "length",
            params: &[],
            facts: FALLIBLE,
            instr: BuiltinInstr::Length,
            calc: Some(CalcRecipe {
                instr: BuiltinInstr::Length,
            }),
            docs: "Return the input length (Unicode code points for strings).",
        },
        scalar_spec!("abs", [], Abs),
        BuiltinSpec {
            name: "empty",
            params: &[],
            facts: Facts {
                input: crate::jq::compiler::analyze::InputUse::Ignored,
                cardinality: crate::jq::compiler::analyze::Cardinality::ZeroOrOne,
                ..PURE
            },
            instr: BuiltinInstr::Empty,
            calc: None,
            docs: "Produce no output.",
        },
        BuiltinSpec {
            name: "error",
            params: &[],
            facts: Facts {
                cardinality: crate::jq::compiler::analyze::Cardinality::ZeroOrOne,
                ..FALLIBLE
            },
            instr: BuiltinInstr::Error,
            calc: None,
            docs: "Raise the input as a runtime error.",
        },
        BuiltinSpec {
            name: "halt",
            params: &[],
            facts: Facts {
                input: crate::jq::compiler::analyze::InputUse::Ignored,
                cardinality: crate::jq::compiler::analyze::Cardinality::ZeroOrOne,
                ..PURE
            },
            instr: BuiltinInstr::Halt,
            calc: None,
            docs: "Stop execution with exit status zero.",
        },
        BuiltinSpec {
            name: "input",
            params: &[],
            facts: Facts {
                cardinality: crate::jq::compiler::analyze::Cardinality::One,
                ..HOST
            },
            instr: BuiltinInstr::Input,
            calc: None,
            docs: "Read from the shared host input cursor.",
        },
        BuiltinSpec {
            name: "env",
            params: &[],
            facts: Facts {
                cardinality: crate::jq::compiler::analyze::Cardinality::One,
                ..HOST
            },
            instr: BuiltinInstr::Env,
            calc: None,
            docs: "Return the host environment.",
        },
        BuiltinSpec {
            name: "input_filename",
            params: &[],
            facts: Facts {
                cardinality: crate::jq::compiler::analyze::Cardinality::One,
                ..HOST
            },
            instr: BuiltinInstr::InputFilename,
            calc: None,
            docs: "Return the name of the file the current input came from, or null.",
        },
        BuiltinSpec {
            name: "input_line_number",
            params: &[],
            facts: Facts {
                cardinality: crate::jq::compiler::analyze::Cardinality::One,
                ..HOST
            },
            instr: BuiltinInstr::InputLineNumber,
            calc: None,
            docs: "Return the current input line number.",
        },
        BuiltinSpec {
            name: "modulemeta",
            params: &[],
            facts: Facts {
                cardinality: crate::jq::compiler::analyze::Cardinality::One,
                ..HOST_WITH_INPUT
            },
            instr: BuiltinInstr::ModuleMeta,
            calc: None,
            docs: "Look up the metadata of the module named by the input.",
        },
        BuiltinSpec {
            name: "now",
            params: &[],
            facts: Facts {
                cardinality: crate::jq::compiler::analyze::Cardinality::One,
                ..HOST
            },
            instr: BuiltinInstr::Now,
            calc: None,
            docs: "Current time as seconds since the epoch.",
        },
        scalar_spec!("gmtime", [], Gmtime),
        scalar_spec!("localtime", [], Localtime),
        scalar_spec!("mktime", [], Mktime),
        scalar_spec!("strftime", [ParamMode::Value], Strftime),
        scalar_spec!("strflocaltime", [ParamMode::Value], Strflocaltime),
        scalar_spec!("strptime", [ParamMode::Value], Strptime),
        BuiltinSpec {
            name: "debug",
            params: &[],
            facts: Facts {
                cardinality: crate::jq::compiler::analyze::Cardinality::One,
                effects: crate::jq::compiler::analyze::Effects {
                    may_error: false,
                    host_io: true,
                    nondeterministic: false,
                },
                ..PURE
            },
            instr: BuiltinInstr::Debug,
            calc: None,
            docs: "Print [\"DEBUG:\", .] to stderr and pass the input through.",
        },
        BuiltinSpec {
            name: "stderr",
            params: &[],
            facts: Facts {
                cardinality: crate::jq::compiler::analyze::Cardinality::One,
                effects: crate::jq::compiler::analyze::Effects {
                    may_error: false,
                    host_io: true,
                    nondeterministic: false,
                },
                ..PURE
            },
            instr: BuiltinInstr::Stderr,
            calc: None,
            docs: "Print . compactly to stderr (no decoration) and pass the input through.",
        },
        BuiltinSpec {
            name: "have_decnum",
            params: &[],
            facts: PURE,
            instr: BuiltinInstr::HaveDecnum,
            calc: None,
            docs: "oneq preserves a numeric literal's exact value via Decimal \
                   until arithmetic is applied, matching jq's own decNum mode \
                   (exact literal round-trip, double-precision arithmetic).",
        },
        BuiltinSpec {
            name: "have_literal_numbers",
            params: &[],
            facts: PURE,
            instr: BuiltinInstr::HaveLiteralNumbers,
            calc: None,
            docs: "oneq doesn't preserve a number literal's original spelling at runtime.",
        },
    ]
}

fn index() -> &'static std::collections::HashMap<(crate::strs::Symbol, usize), BuiltinId> {
    static INDEX: std::sync::OnceLock<
        std::collections::HashMap<(crate::strs::Symbol, usize), BuiltinId>,
    > = std::sync::OnceLock::new();
    INDEX.get_or_init(|| {
        registry()
            .iter()
            .enumerate()
            .map(|(i, s)| ((crate::strs::intern(s.name), s.params.len()), BuiltinId(i)))
            .collect()
    })
}

pub fn lookup_symbol(name: crate::strs::Symbol, arity: usize) -> Option<BuiltinId> {
    index().get(&(name, arity)).copied()
}

pub fn spec(id: BuiltinId) -> &'static BuiltinSpec {
    &registry()[id.0]
}

/// Native streams request filter evaluation through a VM continuation.
#[derive(Clone, Debug)]
pub enum NativeState {
    Pending(fn(&Value, &[Value]) -> Result<Vec<Value>, JqError>),
    Values {
        values: std::rc::Rc<Vec<Value>>,
        next: usize,
    },
    Range {
        next: f64,
        end: f64,
        step: f64,
    },
}

pub(crate) fn range() -> NativeState {
    NativeState::Range {
        next: 0.0,
        end: 0.0,
        step: 1.0,
    }
}

pub enum NativeEvent {
    Output(Value),
    Callback { filter: Closure, input: Value },
    Done,
}

/// Range advancement is infallible after argument validation. Share the
/// floating-point termination rule with the VM's backtracking path.
pub(crate) fn range_next(next: &mut f64, end: f64, step: f64) -> Option<Value> {
    if (step > 0.0 && *next < end) || (step < 0.0 && *next > end) {
        let value = *next;
        *next += step;
        Some(Value::Float(value))
    } else {
        None
    }
}

impl NativeState {
    pub fn resume(
        &mut self,
        _callback_result: Option<Result<Value, JqError>>,
    ) -> Result<NativeEvent, JqError> {
        match self {
            Self::Pending(_) => Err(JqError::InvalidCode("uninitialized native stream".into())),
            Self::Values { values, next } => {
                if let Some(value) = values.get(*next) {
                    *next += 1;
                    Ok(NativeEvent::Output(value.clone()))
                } else {
                    Ok(NativeEvent::Done)
                }
            }
            Self::Range { next, end, step } => Ok(range_next(next, *end, *step)
                .map(NativeEvent::Output)
                .unwrap_or(NativeEvent::Done)),
        }
    }

    pub(crate) fn initialize(&mut self, input: &Value, args: &[Value]) -> Result<(), JqError> {
        match self {
            Self::Pending(create) => {
                *self = Self::Values {
                    values: std::rc::Rc::new(create(input, args)?),
                    next: 0,
                };
            }
            Self::Values { .. } => {
                return Err(JqError::InvalidCode(
                    "native stream already initialized".into(),
                ));
            }
            Self::Range { next, end, step } => {
                *next = if args.len() == 1 {
                    0.0
                } else {
                    scalar::number(&args[0])?
                };
                *end = scalar::number(&args[if args.len() == 1 { 0 } else { 1 }])?;
                *step = if args.len() == 3 {
                    scalar::number(&args[2])?
                } else {
                    1.0
                };
            }
        }
        Ok(())
    }
}
