use std::rc::Rc;

use crate::{data::Value, strs};

/// Implements jq's `modulemeta`: given a module name, resolves and parses it
/// with the same search rules as `import`/`include` (reusing the compiler's
/// own module loader via a throwaway synthetic `import "<name>" as x;`
/// program), then reports `{..the module's own declared metadata.., deps,
/// defs}` - `deps` extends each import's own metadata object (e.g. a
/// `{search: ...}` it wrote) with `as`/`is_data`/`relpath`, and `defs` lists
/// this module's own top-level `name/arity` definitions.
pub fn modulemeta(
    name: &str,
    filter_path: &str,
    module_dirs: &[std::path::PathBuf],
) -> Result<Value, crate::jq::compiler::CompileError> {
    use crate::jq::compiler::{
        CompileError, CompileOptions,
        modules::{DependencyTarget, ImportKind, ModuleCache},
    };
    use crate::jq::parser::pairs::PairTag;

    let mut escaped = String::new();
    crate::data::core::escape::escape_string_json(name, '"', &mut escaped)
        .map_err(|e| CompileError(e.to_string()))?;
    let source = format!("import \"{escaped}\" as x;");
    let options = CompileOptions {
        path: filter_path.to_string(),
        module_dirs: module_dirs.to_vec(),
        ..CompileOptions::default()
    };
    let graph = ModuleCache::default().prepare(&source, &options)?;
    let dependency = graph
        .entry()
        .dependencies
        .first()
        .ok_or_else(|| CompileError("module has no dependencies".into()))?;
    let DependencyTarget::Module(id) = &dependency.target else {
        return Err(CompileError("modulemeta target is a data import".into()));
    };
    let module = graph
        .module(*id)
        .ok_or_else(|| CompileError("module ordering error".into()))?;
    let source = &module.source;

    let mut result = match &source.metadata {
        Some(Value::Object(object)) => object.as_ref().clone(),
        _ => indexmap::IndexMap::new(),
    };

    let dependencies: Vec<Value> = source
        .imports
        .iter()
        .map(|import| {
            let mut object = match &import.metadata {
                Some(Value::Object(object)) => object.as_ref().clone(),
                _ => indexmap::IndexMap::new(),
            };
            let (alias, is_data) = match &import.kind {
                ImportKind::Include => (import.path.clone(), false),
                ImportKind::Module { alias } => (alias.clone(), false),
                ImportKind::Data { alias } => (alias.clone(), true),
            };
            object.insert(strs::keyword_as(), Value::String(alias.into()));
            object.insert(strs::keyword_is_data(), Value::Bool(is_data));
            object.insert(
                strs::keyword_relpath(),
                Value::String(import.path.clone().into()),
            );
            Value::Object(Rc::new(object))
        })
        .collect();

    // Top-level `def name(...): ...;` siblings of the module's root - a
    // defs-only file parses each as a standalone `funcdef` pair, whose
    // semantic children are its params followed by its body and (mirroring
    // `lower_definitions`'s own `children.len() - 2`) one further trailing
    // slot, so the parameter count is `len() - 2`.
    let definitions: Vec<Value> = source
        .root
        .semantic_children()
        .filter(|pair| pair.tag == PairTag::Def)
        .map(|pair| {
            let name = pair.text(&source.files).unwrap_or("");
            let arity = pair.semantic_children().count().saturating_sub(2);
            Value::String(format!("{name}/{arity}").into())
        })
        .collect();

    result.insert(strs::keyword_deps(), Value::Array(Rc::new(dependencies)));
    result.insert(strs::keyword_defs(), Value::Array(Rc::new(definitions)));
    Ok(Value::Object(Rc::new(result)))
}

pub fn environment() -> Value {
    if cfg!(all(target_arch = "wasm32", target_os = "unknown")) {
        return Value::Object(Rc::new(Default::default()));
    }
    Value::Object(Rc::new(
        std::env::vars_os()
            .filter_map(|(k, v)| {
                Some((
                    strs::intern(&k.into_string().ok()?),
                    Value::String(v.into_string().ok()?.into()),
                ))
            })
            .collect(),
    ))
}
