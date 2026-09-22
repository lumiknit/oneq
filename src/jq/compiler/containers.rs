//! Fixed container layouts. Streams retain ordinary collection/cartesian code.
use crate::{
    data,
    jq::{
        builtins,
        ir::{Expr, ExprId, Ir, Literal},
        vm::code::{BuildValue, ContainerPlan},
    },
    strs,
};

pub(super) fn literal(ir: &Ir, id: ExprId) -> Option<data::Value> {
    match &ir.nodes[id.0].expr {
        Expr::Literal(Literal::Value(v)) => Some(v.clone()),
        Expr::Literal(Literal::Number(s)) => data::parse_json_str(s).ok(),
        _ => None,
    }
}

// Deliberately structural: one successful output and no escaping declarations.
// Coarse cardinality facts alone don't prove collection scopes can be removed.
fn single(ir: &Ir, id: ExprId) -> bool {
    match &ir.nodes[id.0].expr {
        Expr::Input | Expr::Literal(_) | Expr::Read(_) => true,
        Expr::ConstPath { base, .. } => single(ir, *base),
        Expr::Pipe(items) => items.iter().all(|id| single(ir, *id)),
        Expr::BuiltinCall { builtin, args } => {
            builtins::spec(*builtin).instr.is_scalar() && args.iter().all(|id| single(ir, *id))
        }
        _ => false,
    }
}

pub(super) fn plan(ir: &Ir, id: ExprId) -> Option<(ContainerPlan, Vec<ExprId>)> {
    let mut dynamic = Vec::new();
    let mut field = |id, stream| {
        if let Some(value) = literal(ir, id) {
            Some(BuildValue::Constant(value))
        } else if stream || single(ir, id) {
            dynamic.push(id);
            Some(BuildValue::Dynamic)
        } else {
            None
        }
    };
    let plan = match &ir.nodes[id.0].expr {
        Expr::Array(inner) => {
            let items = match &ir.nodes[inner.0].expr {
                Expr::Concat(items) => items.as_slice(),
                _ => std::slice::from_ref(inner),
            };
            ContainerPlan::Array(
                items
                    .iter()
                    .map(|id| field(*id, false))
                    .collect::<Option<_>>()?,
            )
        }
        Expr::Object(pairs) => {
            let mut keys = std::collections::HashSet::new();
            let mut fields = Vec::new();
            for (key, value) in pairs {
                let data::Value::String(key) = literal(ir, *key)? else {
                    return None;
                };
                let key = strs::intern(&key);
                if !keys.insert(key) {
                    return None;
                }
                fields.push((key, field(*value, true)?));
            }
            ContainerPlan::Object(fields)
        }
        _ => return None,
    };
    Some((plan, dynamic))
}
