use super::{CompileError, CompileOptions};
use crate::{
    data,
    jq::{
        ir::{BindingId, CallTarget, Expr, ExprId, Ir, Literal, PathStep, Pattern, PatternId},
        vm::code::{Chunk, CodeId, Function, Instruction, SlotKey},
    },
};

#[derive(Default)]
struct Slots {
    keys: Vec<SlotKey>,
    owned: Vec<usize>,
}
impl Slots {
    fn allocate(&mut self, key: SlotKey) -> usize {
        if let Some(slot) = self.keys.iter().position(|k| *k == key) {
            return slot;
        }
        self.keys.push(key);
        self.keys.len() - 1
    }
    fn bind(&mut self, ir: &Ir, id: BindingId) -> Instruction {
        let slot = self.allocate(SlotKey::Local(id));
        self.owned.push(slot);
        Instruction::Bind {
            slot,
            export: ir.export_bindings.contains(&id).then_some(id),
        }
    }
}

/// A nested definition may need an enclosing capture only to pass it on to
/// another closure. Propagate those dependencies before fixing capture offsets.
fn resolve_captures(
    code: &mut [Instruction],
    entry: &mut Vec<SlotKey>,
    functions: &mut [Function],
) {
    let ends: Vec<_> = functions
        .iter()
        .map(|f| f.code.offset)
        .chain([code.len()])
        .collect();
    loop {
        let mut changed = false;
        for scope in 0..=functions.len() {
            let start = if scope == 0 { 0 } else { ends[scope - 1] };
            let end = ends[scope];
            for instruction in &code[start..end] {
                let Instruction::Define { function, .. } = instruction else {
                    continue;
                };
                let callee = functions.iter().find(|f| f.id == *function).unwrap();
                let needed: Vec<_> = callee.captures.iter().map(|i| callee.slots[*i]).collect();
                for key in needed {
                    if scope == 0 {
                        if !entry.contains(&key) {
                            entry.push(key);
                            changed = true;
                        }
                    } else {
                        let caller = &mut functions[scope - 1];
                        if !caller.slots.contains(&key) {
                            caller.captures.push(caller.slots.len());
                            caller.slots.push(key);
                            changed = true;
                        }
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    for scope in 0..=functions.len() {
        let start = if scope == 0 { 0 } else { ends[scope - 1] };
        let keys = if scope == 0 {
            &*entry
        } else {
            &functions[scope - 1].slots
        };
        for instruction in &mut code[start..ends[scope]] {
            if let Instruction::Define {
                function, captures, ..
            } = instruction
            {
                let callee = functions.iter().find(|f| f.id == *function).unwrap();
                *captures = callee
                    .captures
                    .iter()
                    .map(|dst| {
                        (
                            *dst,
                            keys.iter()
                                .position(|key| *key == callee.slots[*dst])
                                .unwrap(),
                        )
                    })
                    .collect();
            }
        }
    }
}

pub(super) fn emit(ir: Ir, entry: ExprId, options: &CompileOptions) -> Result<Chunk, CompileError> {
    if options.calc {
        return Err(CompileError(
            "M4: calc compilation is not implemented".into(),
        ));
    }
    let mut code = Vec::new();
    let mut slots = Slots::default();
    emit_expr(&ir, entry, &mut code, &mut slots)?;
    code.extend([Instruction::Yield, Instruction::Backtrack]);
    let mut functions = Vec::new();
    for definition in &ir.functions {
        let mut scope = Slots::default();
        let self_slot = scope.allocate(SlotKey::Function(definition.id));
        scope.owned.push(self_slot);
        let params = definition
            .params
            .iter()
            .map(|id| {
                let slot = scope.allocate(SlotKey::Filter(*id));
                scope.owned.push(slot);
                slot
            })
            .collect();
        functions.push(Function {
            id: definition.id,
            code: CodeId {
                chunk: 0,
                offset: code.len(),
            },
            params,
            slots: vec![],
            captures: vec![],
            self_slot,
        });
        emit_expr(&ir, definition.body, &mut code, &mut scope)?;
        code.push(Instruction::Return);
        let function = functions.last_mut().unwrap();
        function.captures = (0..scope.keys.len())
            .filter(|i| !scope.owned.contains(i))
            .collect();
        function.slots = scope.keys;
    }
    resolve_captures(&mut code, &mut slots.keys, &mut functions);
    Ok(Chunk {
        slots: slots.keys,
        data_bindings: vec![],
        definition_only: false,
        ir,
        entry,
        code,
        functions,
        calc: vec![],
    })
}
fn emit_expr(
    ir: &Ir,
    entry: ExprId,
    code: &mut Vec<Instruction>,
    slots: &mut Slots,
) -> Result<(), CompileError> {
    match &ir.nodes[entry.0].expr {
        Expr::Input => {}
        Expr::Paths(body) => {
            code.push(Instruction::BeginPath);
            emit_expr(ir, *body, code, slots)?;
            code.push(Instruction::EndPath);
        }
        Expr::Literal(literal) => {
            let value = match literal {
                Literal::Value(value) => value.clone(),
                Literal::Number(raw) => data::parse_json_str(raw).map_err(CompileError)?,
            };
            code.push(Instruction::Load(value));
        }
        Expr::Read(binding) => {
            code.push(Instruction::Read(slots.allocate(SlotKey::Local(*binding))));
        }
        Expr::Pipe(items) => {
            for item in items {
                emit_expr(ir, *item, code, slots)?;
            }
        }
        Expr::Concat(items) => {
            let mut joins = Vec::new();
            for (i, item) in items.iter().enumerate() {
                if i + 1 == items.len() {
                    emit_expr(ir, *item, code, slots)?;
                    break;
                }
                let fork = code.len();
                code.push(Instruction::Fork(0));
                emit_expr(ir, *item, code, slots)?;
                joins.push(code.len());
                code.push(Instruction::Jump(0));
                code[fork] = Instruction::Fork(code.len());
            }
            for jump in joins {
                code[jump] = Instruction::Jump(code.len());
            }
            if items.is_empty() {
                code.push(Instruction::Backtrack);
            }
        }
        Expr::Bind {
            source,
            pattern,
            body,
        } => {
            code.push(Instruction::Push);
            emit_expr(ir, *source, code, slots)?;
            if let Pattern::Alternatives(patterns) = &ir.patterns[pattern.0] {
                code.push(Instruction::Push);
                let mut bindings = Vec::new();
                pattern_bindings(ir, *pattern, &mut bindings);
                let mut joins = Vec::new();
                for (index, pattern) in patterns.iter().enumerate() {
                    let last = index + 1 == patterns.len();
                    let begin = code.len();
                    if !last {
                        code.push(Instruction::BeginDestructure(0));
                    }
                    code.push(Instruction::Pop);
                    code.push(Instruction::Push);
                    code.push(Instruction::Load(crate::data::Value::Null));
                    for binding in &bindings {
                        code.push(slots.bind(ir, *binding));
                    }
                    code.push(Instruction::Pop);
                    emit_pattern(ir, *pattern, code, slots)?;
                    code.push(Instruction::Pop);
                    emit_expr(ir, *body, code, slots)?;
                    joins.push(code.len());
                    code.push(Instruction::Jump(0));
                    if !last {
                        code[begin] = Instruction::BeginDestructure(code.len());
                    }
                }
                for join in joins {
                    code[join] = Instruction::Jump(code.len());
                }
            } else {
                emit_pattern(ir, *pattern, code, slots)?;
                code.push(Instruction::Pop);
                emit_expr(ir, *body, code, slots)?;
            }
        }
        Expr::Scope { functions, body } => {
            for function in functions {
                let slot = slots.allocate(SlotKey::Function(*function));
                slots.owned.push(slot);
                code.push(Instruction::Define {
                    slot,
                    function: *function,
                    captures: vec![],
                });
            }
            emit_expr(ir, *body, code, slots)?;
        }
        Expr::Call { target, args } => {
            let skip = code.len();
            code.push(Instruction::Jump(0));
            let mut offsets = Vec::new();
            for arg in args {
                offsets.push(code.len());
                emit_expr(ir, *arg, code, slots)?;
                code.push(Instruction::Return);
            }
            code[skip] = Instruction::Jump(code.len());
            let key = match target {
                CallTarget::Function(id) => SlotKey::Function(*id),
                CallTarget::FilterParameter(id) => SlotKey::Filter(*id),
            };
            code.push(Instruction::Call(slots.allocate(key), offsets));
        }
        Expr::Reduce {
            source,
            pattern,
            init,
            update,
        }
        | Expr::Foreach {
            source,
            pattern,
            init,
            update,
            ..
        } => {
            code.push(Instruction::Push);
            emit_expr(ir, *init, code, slots)?;
            let begin = code.len();
            code.push(Instruction::BeginFold(0));
            code.push(Instruction::Pop);
            emit_expr(ir, *source, code, slots)?;
            emit_pattern(ir, *pattern, code, slots)?;
            code.push(Instruction::FoldLoad);
            emit_expr(ir, *update, code, slots)?;
            code.push(Instruction::FoldStore);
            if let Expr::Foreach { extract, .. } = &ir.nodes[entry.0].expr {
                emit_expr(ir, *extract, code, slots)?;
                code.push(Instruction::DropFold);
                let end = code.len();
                code.push(Instruction::Jump(0));
                code[begin] = Instruction::BeginFold(code.len());
                code.push(Instruction::EndFold);
                code.push(Instruction::Drop);
                code.push(Instruction::Backtrack);
                code[end] = Instruction::Jump(code.len());
            } else {
                code.push(Instruction::Backtrack);
                code[begin] = Instruction::BeginFold(code.len());
                code.push(Instruction::EndFold);
                code.push(Instruction::Drop);
            }
        }
        Expr::Label { label, body } => {
            code.push(Instruction::BeginLabel(*label));
            emit_expr(ir, *body, code, slots)?;
            code.push(Instruction::EndLabel);
        }
        Expr::Break(label) => code.push(Instruction::Break(*label)),
        Expr::Try { body, handler } => {
            let begin = code.len();
            code.push(Instruction::BeginTry(0));
            emit_expr(ir, *body, code, slots)?;
            code.push(Instruction::EndTry);
            let end = code.len();
            code.push(Instruction::Jump(0));
            code[begin] = Instruction::BeginTry(code.len());
            emit_expr(ir, *handler, code, slots)?;
            code[end] = Instruction::Jump(code.len());
        }
        Expr::Alternative { lhs, rhs } => {
            let begin = code.len();
            code.push(Instruction::BeginAlternative(0));
            emit_expr(ir, *lhs, code, slots)?;
            code.push(Instruction::AlternativeItem);
            let end = code.len();
            code.push(Instruction::Jump(0));
            code[begin] = Instruction::BeginAlternative(code.len());
            emit_expr(ir, *rhs, code, slots)?;
            code[end] = Instruction::Jump(code.len());
        }
        Expr::If { condition, yes, no } => {
            code.push(Instruction::Push);
            emit_expr(ir, *condition, code, slots)?;
            let branch = code.len();
            code.push(Instruction::JumpFalse(0));
            code.push(Instruction::Pop);
            emit_expr(ir, *yes, code, slots)?;
            let end = code.len();
            code.push(Instruction::Jump(0));
            code[branch] = Instruction::JumpFalse(code.len());
            code.push(Instruction::Pop);
            emit_expr(ir, *no, code, slots)?;
            code[end] = Instruction::Jump(code.len());
        }
        Expr::Path { base, steps } => {
            code.push(Instruction::Push);
            emit_expr(ir, *base, code, slots)?;
            for step in steps {
                match step {
                    PathStep::Iterate => code.push(Instruction::Iterate),
                    PathStep::Index(index) => {
                        code.push(Instruction::Push);
                        code.push(Instruction::LoadSaved(1));
                        // The key expression is evaluated as a plain value,
                        // never as a path in its own right (even a nested
                        // `path(...)` inside it just extracts a value) - see
                        // `SuspendPath`/`ResumePath`.
                        code.push(Instruction::SuspendPath);
                        emit_expr(ir, *index, code, slots)?;
                        code.push(Instruction::ResumePath);
                        code.push(Instruction::Index);
                    }
                    PathStep::Slice { start, end } => {
                        code.push(Instruction::Push);
                        code.push(Instruction::LoadSaved(1));
                        code.push(Instruction::SuspendPath);
                        if let Some(start) = start {
                            emit_expr(ir, *start, code, slots)?;
                        } else {
                            code.push(Instruction::Load(crate::data::Value::Null));
                        }
                        code.push(Instruction::ResumePath);
                        code.push(Instruction::Push);
                        code.push(Instruction::LoadSaved(2));
                        code.push(Instruction::SuspendPath);
                        if let Some(end) = end {
                            emit_expr(ir, *end, code, slots)?;
                        } else {
                            code.push(Instruction::Load(crate::data::Value::Null));
                        }
                        code.push(Instruction::ResumePath);
                        code.push(Instruction::Slice);
                    }
                }
            }
            code.push(Instruction::Drop);
        }
        Expr::Array(inner) => {
            let begin = code.len();
            code.push(Instruction::BeginCollect(0));
            emit_expr(ir, *inner, code, slots)?;
            code.push(Instruction::CollectItem);
            code.push(Instruction::Backtrack);
            let end = code.len();
            code[begin] = Instruction::BeginCollect(end);
            code.push(Instruction::EndCollect);
        }
        Expr::Object(pairs) => {
            code.push(Instruction::Push);
            let mut depth = 0;
            for (key, value) in pairs {
                for id in [key, value] {
                    code.push(Instruction::LoadSaved(depth));
                    emit_expr(ir, *id, code, slots)?;
                    code.push(Instruction::Push);
                    depth += 1;
                }
            }
            code.push(Instruction::MakeObject(pairs.len()));
        }
        Expr::BuiltinCall { builtin, args } => {
            let instr = crate::jq::builtins::spec(*builtin).instr;
            if instr.is_infix_operator()
                && let Expr::Literal(literal) = &ir.nodes[args[1].0].expr
            {
                let right = match literal {
                    Literal::Value(value) => value.clone(),
                    Literal::Number(raw) => data::parse_json_str(raw).map_err(CompileError)?,
                };
                // A literal yields once and has no effects. Keep it in code,
                // even when the left operand yields, errors, or suspends.
                emit_expr(ir, args[0], code, slots)?;
                code.push(Instruction::InfixConst(instr.op2(), right));
                return Ok(());
            }
            // Ordinary multi-arg calls (including 2-arg natives like
            // `range(a;b)`) nest left-to-right: the leftmost arg is the
            // outer loop (varies slowest), matching plain jq-defined
            // functions (`def f($a;$b): ...; f(1,2;10,20)` yields
            // [1,10],[1,20],[2,10],[2,20]).
            //
            // jq's built-in infix operators (`+`, `-`, `==`, ...) are a
            // documented exception: `gen_binop` in jq's own compiler nests
            // the *right* operand outer instead (`(1,2)+(10,20)` yields
            // 11,12,21,22 - right varies slowest), so those are emitted
            // right-to-left here to match.
            //
            // Either way, each arg's code sees the original input, and
            // `BuiltinCall` restores source order from its LIFO pops
            // regardless of which order we pushed in.
            if !args.is_empty() {
                code.push(Instruction::Push);
            }
            let order: Vec<usize> = if crate::jq::builtins::spec(*builtin).is_infix_operator() {
                (0..args.len()).rev().collect()
            } else {
                (0..args.len()).collect()
            };
            for (depth, idx) in order.into_iter().enumerate() {
                code.push(Instruction::LoadSaved(depth));
                emit_expr(ir, args[idx], code, slots)?;
                code.push(Instruction::Push);
            }
            code.push(match args.len() {
                0 => Instruction::BuiltinCall0(instr.op0()),
                1 => Instruction::BuiltinCall1(instr.op1()),
                2 => Instruction::BuiltinCall2(instr.op2()),
                3 => Instruction::BuiltinCall3(instr.op3()),
                _ => return Err(CompileError("builtin arity exceeds VM limit".into())),
            });
        }
    }
    Ok(())
}

fn emit_pattern(
    ir: &Ir,
    id: PatternId,
    code: &mut Vec<Instruction>,
    slots: &mut Slots,
) -> Result<(), CompileError> {
    match &ir.patterns[id.0] {
        Pattern::Binding(id) => code.push(slots.bind(ir, *id)),
        Pattern::Array(items) => {
            for (index, pattern) in items.iter().enumerate() {
                code.push(Instruction::Push);
                code.push(Instruction::Push);
                code.push(Instruction::Load(crate::data::Value::int(index as i64)));
                code.push(Instruction::Index);
                emit_pattern(ir, *pattern, code, slots)?;
                code.push(Instruction::Pop);
            }
        }
        Pattern::Object(fields) => {
            for (key, pattern) in fields {
                code.push(Instruction::Push);
                code.push(Instruction::Push);
                emit_expr(ir, *key, code, slots)?;
                code.push(Instruction::Index);
                emit_pattern(ir, *pattern, code, slots)?;
                code.push(Instruction::Pop);
            }
        }
        Pattern::Alternatives(_) => {
            return Err(CompileError(
                "pattern alternatives are not implemented yet".into(),
            ));
        }
    }
    Ok(())
}

fn pattern_bindings(ir: &Ir, pattern: PatternId, bindings: &mut Vec<crate::jq::ir::BindingId>) {
    match &ir.patterns[pattern.0] {
        Pattern::Binding(id) => {
            if !bindings.contains(id) {
                bindings.push(*id);
            }
        }
        Pattern::Array(patterns) | Pattern::Alternatives(patterns) => {
            for pattern in patterns {
                pattern_bindings(ir, *pattern, bindings);
            }
        }
        Pattern::Object(fields) => {
            for (_, pattern) in fields {
                pattern_bindings(ir, *pattern, bindings);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_ir_bindings_get_dense_scope_slots() {
        let mut ir = Ir::default();
        let input = ir.push(Expr::Input, None);
        let ids = [BindingId(3), BindingId(100), BindingId(12784)];
        let reads = ids
            .iter()
            .map(|id| ir.push(Expr::Read(*id), None))
            .collect();
        let mut body = ir.push(Expr::Concat(reads), None);
        for id in ids.into_iter().rev() {
            let pattern = PatternId(ir.patterns.len());
            ir.patterns.push(Pattern::Binding(id));
            body = ir.push(
                Expr::Bind {
                    source: input,
                    pattern,
                    body,
                },
                None,
            );
        }
        let chunk = emit(ir, body, &CompileOptions::default()).unwrap();
        assert_eq!(chunk.slots, ids.map(SlotKey::Local));
        let reads: Vec<_> = chunk
            .code
            .iter()
            .filter_map(|op| match op {
                Instruction::Read(slot) => Some(*slot),
                _ => None,
            })
            .collect();
        assert_eq!(reads, [0, 1, 2]);
    }

    #[test]
    fn each_function_allocates_its_own_filter_and_local_slots() {
        use crate::jq::ir::{FunctionDef, FunctionId};
        let mut ir = Ir::default();
        let entry = ir.push(Expr::Input, None);
        for (function, param, binding) in [(2, 100, 12784), (900, 70000, 90000)] {
            let read = ir.push(Expr::Read(BindingId(binding)), None);
            let source = ir.push(
                Expr::Call {
                    target: CallTarget::FilterParameter(BindingId(param)),
                    args: vec![],
                },
                None,
            );
            let pattern = PatternId(ir.patterns.len());
            ir.patterns.push(Pattern::Binding(BindingId(binding)));
            let body = ir.push(
                Expr::Bind {
                    source,
                    pattern,
                    body: read,
                },
                None,
            );
            ir.functions.push(FunctionDef {
                id: FunctionId(function),
                params: vec![BindingId(param)],
                body,
            });
        }
        let chunk = emit(ir, entry, &CompileOptions::default()).unwrap();
        assert!(chunk.slots.is_empty());
        for function in chunk.functions {
            assert_eq!(function.self_slot, 0);
            assert_eq!(function.params, [1]);
            assert_eq!(function.slots.len(), 3);
            assert!(function.captures.is_empty());
        }
    }
}
