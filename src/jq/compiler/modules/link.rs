//! Hygienic linking: resolve each module independently, then execute declarations
//! in dependency order. Aliases map to identities, never rewritten function bodies.
use super::{DependencyTarget, ImportKind, ModuleGraph};
use crate::{
    jq::{
        compiler::{
            CompileError, CompileOptions, emit, lower,
            templates::{self, TemplateStore},
        },
        ir::{Expr, Ir, ModuleId},
        symbols::{Names, Symbols},
        vm::code::Chunk,
    },
    strs,
};
use std::collections::HashMap;

pub fn compile(
    graph: &ModuleGraph,
    options: &CompileOptions,
    symbols: &mut Symbols,
    templates: &TemplateStore,
) -> Result<(Chunk, TemplateStore), CompileError> {
    let initial = symbols.names.clone();
    let mut ir = Ir::default();
    let mut data_bindings = Vec::new();
    let mut exports: HashMap<ModuleId, Names> = HashMap::new();
    let mut entries = Vec::new();
    for &id in &graph.order {
        let module = graph.module(id).unwrap();
        let is_entry = id == graph.entry().source.id;
        symbols.names = if is_entry {
            initial.clone()
        } else {
            // Non-entry modules still need the prelude's jq-defined functions
            // (`add`, `map`, `_assign`, ...) in scope, just not each other's
            // private definitions, so start from `initial.functions` too.
            Names {
                bindings: initial.bindings.clone(),
                functions: initial.functions.clone(),
                ..Names::default()
            }
        };
        for dependency in &module.dependencies {
            match (&dependency.import.kind, &dependency.target) {
                (ImportKind::Data { alias }, DependencyTarget::Data(data)) => {
                    let binding = symbols.declare(
                        strs::intern(&format!("{alias}::{alias}")),
                        dependency.import.span.clone(),
                    );
                    symbols.bindings[binding.0].module = id;
                    symbols.names.bindings.insert(strs::intern(alias), binding);
                    data_bindings.push((binding, data.value.clone()));
                }
                (kind, DependencyTarget::Module(target)) => {
                    let names = exports
                        .get(target)
                        .ok_or_else(|| CompileError("module ordering error".into()))?;
                    for ((name, arity), function) in &names.functions {
                        let name = strs::resolve(*name).unwrap_or("");
                        let name = match kind {
                            ImportKind::Module { alias } => format!("{alias}::{name}"),
                            ImportKind::Include => name.to_string(),
                            _ => unreachable!(),
                        };
                        symbols
                            .names
                            .functions
                            .insert((strs::intern(&name), *arity), function.clone());
                    }
                }
                _ => return Err(CompileError("invalid dependency kind".into())),
            }
        }
        if is_entry && options.repl {
            for target in symbols.names.functions.values() {
                if let crate::jq::ir::CallTarget::Function(function) = target {
                    ir.export_functions.push(*function);
                }
            }
        }
        let before_bindings = ir.export_bindings.len();
        let before_functions = ir.export_functions.len();
        let imported = std::rc::Rc::make_mut(&mut ir.files)
            .import_pairs(
                &module.source.files,
                std::slice::from_ref(&module.source.root),
            )
            .map_err(CompileError)?;
        let module_options = CompileOptions {
            path: module.source.path.to_string_lossy().into_owned(),
            // Module definition-only continuations end in identity, then the next
            // module's declarations execute. They are not user output entries.
            repl: !is_entry || options.repl,
            entry: is_entry,
            ..options.clone()
        };
        let entry = lower::lower(&mut ir, &imported[0], &module_options, symbols)?;
        entries.push(entry);
        if !is_entry {
            let mut names = symbols.names.clone();
            names.bindings.clear();
            // Names imported under a namespace, and prelude functions
            // inherited rather than defined by this module, stay out of
            // its exports.
            names.functions.retain(|key, _| {
                !strs::resolve(key.0).unwrap_or("").contains("::")
                    && !initial.functions.contains_key(key)
            });
            exports.insert(id, names);
            ir.export_bindings.truncate(before_bindings);
            ir.export_functions.truncate(before_functions);
        }
    }
    let mut entry = ir.push(Expr::Pipe(entries), None);
    let templates = if options.inline {
        templates::optimize(&mut ir, &mut entry, symbols, templates)
    } else {
        TemplateStore::default()
    };
    let mut chunk = emit::emit(ir, entry, options)?;
    chunk.data_bindings = data_bindings;
    chunk.definition_only = options.repl && definition_only(&graph.entry().source.root);
    if !options.repl {
        symbols.names = initial;
    }
    Ok((chunk, templates))
}

fn definition_only(pair: &crate::jq::parser::pairs::Pair) -> bool {
    use crate::jq::parser::pairs::PairTag;
    match pair.tag {
        PairTag::Empty | PairTag::Module | PairTag::Import | PairTag::Include => true,
        PairTag::Def | PairTag::Root => pair
            .semantic_children()
            .next_back()
            .is_none_or(definition_only),
        _ => false,
    }
}
