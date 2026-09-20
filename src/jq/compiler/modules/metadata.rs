//! Static module metadata, independent of the not-yet-complete filter compiler.
use super::*;

pub(super) fn object(pair: &Pair, files: &FileSet) -> Result<Value, CompileError> {
    let value = constant(pair, files)?;
    if !matches!(value, Value::Object(_)) {
        return Err(CompileError(
            "module/import metadata must be an object".into(),
        ));
    }
    Ok(value)
}
pub(super) fn constant(pair: &Pair, files: &FileSet) -> Result<Value, CompileError> {
    let text = pair.text(files).unwrap_or("");
    let children: Vec<_> = pair.semantic_children().collect();
    match pair.tag {
        PairTag::String if children.iter().all(|p| p.tag == PairTag::StringChunk) => {
            let raw: String = children.iter().map(|p| p.text(files).unwrap_or("")).collect();
            data::parse_json_str(&format!("\"{raw}\"")).map_err(CompileError)
        }
        PairTag::Int | PairTag::Float => data::parse_json_str(text).map_err(CompileError),
        PairTag::Invoke if children.is_empty() => match text {
            "null" => Ok(Value::Null), "true" => Ok(Value::Bool(true)), "false" => Ok(Value::Bool(false)),
            _ => Err(CompileError("module metadata must be constant".into())),
        },
        PairTag::Array => {
            let mut values = Vec::new();
            for child in children { array_items(child, files, &mut values)?; }
            Ok(Value::Array(Rc::new(values)))
        }
        PairTag::Object => {
            let mut object = indexmap::IndexMap::new();
            for fields in children.as_chunks::<2>().0 {
                let Value::String(key) = constant(fields[0], files)? else { return Err(CompileError("metadata object key must be a string".into())); };
                object.insert(strs::intern(&key), constant(fields[1], files)?);
            }
            Ok(Value::Object(Rc::new(object)))
        }
        _ => Err(CompileError("module metadata currently requires literal constants (computed metadata is not implemented)".into())),
    }
}
fn array_items(pair: &Pair, files: &FileSet, values: &mut Vec<Value>) -> Result<(), CompileError> {
    if pair.tag == PairTag::Invoke && pair.text(files) == Some(",") {
        for child in pair.semantic_children() {
            array_items(child, files, values)?;
        }
    } else {
        values.push(constant(pair, files)?);
    }
    Ok(())
}
pub(super) fn search_paths(metadata: Option<&Value>) -> Result<Vec<Option<PathBuf>>, CompileError> {
    let Some(Value::Object(object)) = metadata else {
        return Ok(vec![]);
    };
    let Some(search) = object.get(&strs::keyword_search()) else {
        return Ok(vec![]);
    };
    let items: &[Value] = match search {
        Value::Array(items) => items.as_slice(),
        value => std::slice::from_ref(value),
    };
    let mut paths = Vec::new();
    for item in items {
        match item {
            Value::Null => {
                paths.push(None);
                break;
            }
            Value::String(path) if path.is_empty() => {
                paths.push(None);
                break;
            }
            Value::String(path) => paths.push(Some(PathBuf::from(path.to_string()))),
            _ => {
                return Err(CompileError(
                    "metadata search must contain strings or a null terminator".into(),
                ));
            }
        }
    }
    Ok(paths)
}
