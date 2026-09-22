//! Resolved syntax. Names are diagnostic information; references use typed IDs.
use super::parser::pairs::{FileSet, SpanPos};
use crate::data::Value;
use std::rc::Rc;

macro_rules! ids {
    ($($name:ident),* $(,)?) => {$ (
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub struct $name(pub(crate) usize);
    )*};
}
ids!(
    BindingId, FunctionId, LabelId, ModuleId, ExprId, BuiltinId, PatternId
);

#[derive(Clone, Debug)]
pub enum Literal {
    Value(Value),
    /// Keep the spelling until the number representation is made jq-compatible.
    Number(String),
}

#[derive(Clone, Debug)]
pub enum CallTarget {
    Function(FunctionId),
    FilterParameter(BindingId),
}

#[derive(Clone, Debug)]
pub enum Expr {
    Input,
    Paths(ExprId),
    Literal(Literal),
    Read(BindingId),
    Pipe(Vec<ExprId>),
    Concat(Vec<ExprId>),
    Bind {
        source: ExprId,
        pattern: PatternId,
        body: ExprId,
    },
    Call {
        target: CallTarget,
        args: Vec<ExprId>,
    },
    BuiltinCall {
        builtin: BuiltinId,
        args: Vec<ExprId>,
    },
    Scope {
        functions: Vec<FunctionId>,
        body: ExprId,
    },
    Array(ExprId),
    Last(ExprId),
    Object(Vec<(ExprId, ExprId)>),
    Path {
        base: ExprId,
        steps: Vec<PathStep>,
    },
    /// Interned object keys; no key expressions or runtime interning.
    ConstPath {
        base: ExprId,
        steps: Vec<crate::strs::Symbol>,
    },
    If {
        condition: ExprId,
        yes: ExprId,
        no: ExprId,
    },
    Alternative {
        lhs: ExprId,
        rhs: ExprId,
    },
    Try {
        body: ExprId,
        handler: ExprId,
    },
    Reduce {
        source: ExprId,
        pattern: PatternId,
        init: ExprId,
        update: ExprId,
    },
    Foreach {
        source: ExprId,
        pattern: PatternId,
        init: ExprId,
        update: ExprId,
        extract: ExprId,
    },
    Label {
        label: LabelId,
        body: ExprId,
    },
    Break(LabelId),
}

#[derive(Clone, Debug)]
pub enum PathStep {
    Index(ExprId),
    Key(crate::strs::Symbol),
    Iterate,
    Slice {
        start: Option<ExprId>,
        end: Option<ExprId>,
    },
}

#[derive(Clone, Debug)]
pub enum Pattern {
    Binding(BindingId),
    Array(Vec<PatternId>),
    Object(Vec<(ExprId, PatternId)>),
    Alternatives(Vec<PatternId>),
}

#[derive(Clone, Debug)]
pub struct Node {
    pub expr: Expr,
    pub span: Option<SpanPos>,
}

#[derive(Clone, Debug)]
pub struct FunctionDef {
    pub id: FunctionId,
    pub params: Vec<BindingId>,
    pub body: ExprId,
}

#[derive(Clone, Debug, Default)]
pub struct Ir {
    pub files: Rc<FileSet>,
    pub nodes: Vec<Node>,
    pub patterns: Vec<Pattern>,
    pub functions: Vec<FunctionDef>,
    pub export_bindings: Vec<BindingId>,
    pub export_functions: Vec<FunctionId>,
}

impl Ir {
    pub(crate) fn push(&mut self, expr: Expr, span: Option<SpanPos>) -> ExprId {
        let pipe = matches!(&expr, Expr::Pipe(_));
        let expr = match expr {
            Expr::Pipe(items) | Expr::Concat(items) => {
                // Preserve order: an empty filter cannot erase earlier effects.
                let mut flat = Vec::new();
                for id in items {
                    match &self.nodes[id.0].expr {
                        Expr::Input if pipe => {}
                        Expr::Pipe(children) if pipe => flat.extend_from_slice(children),
                        Expr::Concat(children) if !pipe => flat.extend_from_slice(children),
                        _ => flat.push(id),
                    }
                }
                if flat.len() == 1 {
                    return flat[0];
                }
                if pipe && flat.is_empty() {
                    Expr::Input
                } else if pipe {
                    Expr::Pipe(flat)
                } else {
                    Expr::Concat(flat)
                }
            }
            Expr::Path { base, steps } if steps.is_empty() => return base,
            Expr::Scope { functions, body } if functions.is_empty() => return body,
            other => other,
        };
        let id = ExprId(self.nodes.len());
        self.nodes.push(Node { expr, span });
        id
    }
}
