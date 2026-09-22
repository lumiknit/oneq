//! Parse dependencies before name resolution. Cached files are immutable Session snapshots.
//!
//! Each source owns its `FileSet`: sharing a parsed module never requires relocating
//! Pair spans. Dependency edges (aliases/search metadata) belong to a preparation,
//! not to the file cache, so a later compile can use a different search path.
pub(super) mod link;
mod metadata;

use super::{CompileError, CompileOptions};
use crate::{
    data::{self, Value},
    io::Input,
    jq::{
        ir::ModuleId,
        parser::{
            self,
            pairs::{FileSet, Pair, PairTag, SpanPos},
        },
    },
    strs,
};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
    rc::Rc,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportKind {
    Include,
    Module { alias: String },
    Data { alias: String },
}
#[derive(Clone, Debug)]
pub struct Import {
    pub path: String,
    pub kind: ImportKind,
    pub metadata: Option<Value>,
    pub span: Option<SpanPos>,
}
#[derive(Debug)]
pub struct ParsedModule {
    pub id: ModuleId,
    pub path: PathBuf,
    pub canonical_path: Option<PathBuf>,
    pub files: FileSet,
    pub root: Pair,
    pub metadata: Option<Value>,
    pub imports: Vec<Import>,
}
#[derive(Debug)]
pub struct ParsedData {
    pub path: PathBuf,
    pub canonical_path: PathBuf,
    /// jq data imports slurp the complete JSON stream, including an empty stream.
    pub value: Value,
}
#[derive(Clone, Debug)]
pub enum DependencyTarget {
    Module(ModuleId),
    Data(Rc<ParsedData>),
}
#[derive(Clone, Debug)]
pub struct Dependency {
    pub import: Import,
    pub target: DependencyTarget,
}
#[derive(Debug)]
pub struct Module {
    pub source: Rc<ParsedModule>,
    pub dependencies: Vec<Dependency>,
}
#[derive(Debug)]
pub struct ModuleGraph {
    /// Entry first, followed by unique modules in depth-first discovery order.
    pub modules: Vec<Module>,
    /// Dependency-first order, recorded by the loading DFS and reused by linking.
    pub(crate) order: Vec<ModuleId>,
}
impl ModuleGraph {
    #[must_use]
    pub fn entry(&self) -> &Module {
        &self.modules[0]
    }
    #[must_use]
    pub fn module(&self, id: ModuleId) -> Option<&Module> {
        self.modules.iter().find(|m| m.source.id == id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum FileKind {
    Module,
    Data,
}
impl FileKind {
    const fn suffix(self) -> &'static str {
        match self {
            Self::Module => "jq",
            Self::Data => "json",
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct ModuleCache {
    modules: HashMap<PathBuf, Rc<ParsedModule>>,
    data: HashMap<PathBuf, Rc<ParsedData>>,
    // Retain previously resolved spellings too: a cached file may have been removed.
    paths: HashMap<(PathBuf, FileKind), PathBuf>,
    next_module: usize,
}
impl Default for ModuleCache {
    fn default() -> Self {
        Self {
            modules: HashMap::new(),
            data: HashMap::new(),
            paths: HashMap::new(),
            next_module: 1,
        }
    }
}
impl ModuleCache {
    pub(crate) fn prepare(
        &mut self,
        source: &str,
        options: &CompileOptions,
    ) -> Result<ModuleGraph, CompileError> {
        // Cache publication is atomic for dependency loading. A missing or broken
        // dependency can be repaired and retried without keeping half a graph.
        let mut pending = self.clone();
        let graph = pending.resolve(source, options)?;
        *self = pending;
        Ok(graph)
    }
    fn resolve(
        &mut self,
        source: &str,
        options: &CompileOptions,
    ) -> Result<ModuleGraph, CompileError> {
        let path = PathBuf::from(&options.path);
        let canonical = if is_virtual(&path) {
            None
        } else {
            fs::canonicalize(&path).ok()
        };
        let root = Rc::new(parse_module(ModuleId(0), path, canonical, source)?);
        let mut graph = ModuleGraph {
            modules: vec![Module {
                source: root,
                dependencies: vec![],
            }],
            order: vec![],
        };
        // In-memory programs do not need a working directory (or a filesystem).
        if graph.entry().source.imports.is_empty() {
            graph.order.push(ModuleId(0));
            return Ok(graph);
        }
        if cfg!(all(target_arch = "wasm32", target_os = "unknown")) {
            return Err(CompileError(
                "import/include are not supported in WASM".into(),
            ));
        }
        let cwd =
            std::env::current_dir().map_err(|e| CompileError(format!("working directory: {e}")))?;
        let mut known = HashMap::new();
        if let Some(path) = &graph.entry().source.canonical_path {
            known.insert(path.clone(), 0);
        }
        // An explicit DFS stack prevents deeply nested imports from consuming the Rust stack.
        let mut stack = vec![(0usize, 0usize)];
        let mut complete = HashSet::new();
        while let Some(&(index, next)) = stack.last() {
            if next == graph.modules[index].source.imports.len() {
                graph.order.push(graph.modules[index].source.id);
                complete.insert(index);
                stack.pop();
                continue;
            }
            stack.last_mut().unwrap().1 += 1;
            let importer = graph.modules[index].source.clone();
            let import = importer.imports[next].clone();
            let kind = match import.kind {
                ImportKind::Data { .. } => FileKind::Data,
                _ => FileKind::Module,
            };
            let (path, canonical) = self
                .find(&importer, &import, kind, options, &cwd)
                .map_err(|e| import_error(&importer, &import, e))?;
            if kind == FileKind::Data {
                let data = self
                    .load_data(path, canonical)
                    .map_err(|e| import_error(&importer, &import, e))?;
                graph.modules[index].dependencies.push(Dependency {
                    import,
                    target: DependencyTarget::Data(data),
                });
                continue;
            }
            let target_index = if let Some(&found) = known.get(&canonical) {
                found
            } else {
                let source = self
                    .load_module(path, canonical.clone())
                    .map_err(|e| import_error(&importer, &import, e))?;
                let found = graph.modules.len();
                known.insert(canonical, found);
                graph.modules.push(Module {
                    source,
                    dependencies: vec![],
                });
                found
            };
            if stack.iter().any(|&(active, _)| active == target_index) {
                let mut chain: Vec<_> = stack
                    .iter()
                    .map(|&(i, _)| graph.modules[i].source.path.display().to_string())
                    .collect();
                chain.push(
                    graph.modules[target_index]
                        .source
                        .path
                        .display()
                        .to_string(),
                );
                return Err(import_error(
                    &importer,
                    &import,
                    CompileError(format!("cyclic import: {}", chain.join(" -> "))),
                ));
            }
            let target = graph.modules[target_index].source.id;
            graph.modules[index].dependencies.push(Dependency {
                import,
                target: DependencyTarget::Module(target),
            });
            if !complete.contains(&target_index) {
                stack.push((target_index, 0));
            }
        }
        Ok(graph)
    }
    fn load_module(
        &mut self,
        path: PathBuf,
        canonical: PathBuf,
    ) -> Result<Rc<ParsedModule>, CompileError> {
        if let Some(source) = self.modules.get(&canonical) {
            return Ok(source.clone());
        }
        let text = fs::read_to_string(&canonical)
            .map_err(|e| CompileError(format!("{}: {e}", path.display())))?;
        let source = Rc::new(parse_module(
            ModuleId(self.next_module),
            path,
            Some(canonical.clone()),
            &text,
        )?);
        self.next_module += 1;
        self.modules.insert(canonical, source.clone());
        Ok(source)
    }
    fn load_data(
        &mut self,
        path: PathBuf,
        canonical: PathBuf,
    ) -> Result<Rc<ParsedData>, CompileError> {
        if let Some(data) = self.data.get(&canonical) {
            return Ok(data.clone());
        }
        let text = fs::read_to_string(&canonical)
            .map_err(|e| CompileError(format!("{}: {e}", path.display())))?;
        let parser = data::json_parser(Input::new_str(&text));
        let values = data::ValueBuilder::new(parser, data::builder::StreamOption::default())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| CompileError(format!("{}: {e}", path.display())))?;
        let data = Rc::new(ParsedData {
            path,
            canonical_path: canonical.clone(),
            value: Value::Array(Rc::new(values)),
        });
        self.data.insert(canonical, data.clone());
        Ok(data)
    }
    fn find(
        &mut self,
        importer: &ParsedModule,
        import: &Import,
        kind: FileKind,
        options: &CompileOptions,
        cwd: &Path,
    ) -> Result<(PathBuf, PathBuf), CompileError> {
        validate_import_path(&import.path)?;
        let base = if let Some(path) = &importer.canonical_path {
            path.parent().unwrap_or(cwd).to_path_buf()
        } else if is_virtual(&importer.path) {
            cwd.to_path_buf()
        } else {
            cwd.join(&importer.path)
                .parent()
                .unwrap_or(cwd)
                .to_path_buf()
        };
        let mut search = metadata::search_paths(import.metadata.as_ref())?;
        let explicit_search = matches!(import.metadata.as_ref(), Some(Value::Object(object))
            if object.contains_key(&strs::keyword_search()));
        if !explicit_search && options.module_dirs.is_empty() {
            search.extend([
                Some(PathBuf::from("~/.jq")),
                Some(PathBuf::from("$ORIGIN/../lib/jq")),
                Some(PathBuf::from("$ORIGIN/../lib")),
                Some(PathBuf::from(".")),
            ]);
        } else if !explicit_search {
            search.extend(options.module_dirs.iter().cloned().map(Some));
        }
        let mut attempted = Vec::new();
        for directory in search {
            let Some(directory) = directory else {
                break;
            };
            if directory.as_os_str().is_empty() {
                break;
            }
            let Some(directory) = expand_search(&directory, &base)? else {
                continue;
            };
            let relative = Path::new(&import.path);
            let filename = relative
                .file_name()
                .ok_or_else(|| CompileError("empty module filename".into()))?;
            let mut direct = directory.join(relative).into_os_string();
            direct.push(format!(".{}", kind.suffix()));
            let mut nested = directory.join(relative).join(filename).into_os_string();
            nested.push(format!(".{}", kind.suffix()));
            for candidate in [PathBuf::from(direct), PathBuf::from(nested)] {
                attempted.push(candidate.display().to_string());
                if let Some(canonical) = self.paths.get(&(candidate.clone(), kind)) {
                    let cached = match kind {
                        FileKind::Module => self.modules.contains_key(canonical),
                        FileKind::Data => self.data.contains_key(canonical),
                    };
                    if cached {
                        return Ok((candidate, canonical.clone()));
                    }
                }
                match fs::canonicalize(&candidate) {
                    Ok(canonical) => {
                        if !canonical.is_file() {
                            continue;
                        }
                        self.paths
                            .insert((candidate.clone(), kind), canonical.clone());
                        return Ok((candidate, canonical));
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                        ) => {}
                    Err(error) => {
                        return Err(CompileError(format!("{}: {error}", candidate.display())));
                    }
                }
            }
        }
        Err(CompileError(format!(
            "{} not found: {} (searched: {})",
            kind.suffix(),
            import.path,
            attempted.join(", ")
        )))
    }
}

fn parse_module(
    id: ModuleId,
    path: PathBuf,
    canonical_path: Option<PathBuf>,
    text: &str,
) -> Result<ParsedModule, CompileError> {
    let (files, root) = parser::parse_pairs(path.to_string_lossy().into_owned(), text)
        .map_err(|e| CompileError(format!("{}: {e}", path.display())))?;
    let mut metadata_value = None;
    let mut imports = Vec::new();
    for pair in root.semantic_children() {
        let children: Vec<_> = pair.semantic_children().collect();
        match pair.tag {
            PairTag::Module => metadata_value = Some(metadata::object(children[0], &files)?),
            PairTag::Include | PairTag::Import => {
                let Value::String(name) = metadata::constant(children[0], &files)? else {
                    return Err(CompileError(format!(
                        "{}: import path must be a constant string",
                        path.display()
                    )));
                };
                let alias = pair.text(&files).unwrap_or("");
                let kind = if pair.tag == PairTag::Include {
                    ImportKind::Include
                } else if let Some(name) = alias.strip_prefix('$') {
                    ImportKind::Data { alias: name.into() }
                } else {
                    ImportKind::Module {
                        alias: alias.into(),
                    }
                };
                let metadata = children
                    .get(1)
                    .map(|p| metadata::object(p, &files))
                    .transpose()?;
                imports.push(Import {
                    path: name.to_string(),
                    kind,
                    metadata,
                    span: pair.span.clone(),
                });
            }
            _ => {}
        }
    }
    Ok(ParsedModule {
        id,
        path,
        canonical_path,
        files,
        root,
        metadata: metadata_value,
        imports,
    })
}
fn is_virtual(path: &Path) -> bool {
    path.to_str()
        .is_some_and(|s| s.starts_with('<') && s.ends_with('>'))
}
fn validate_import_path(path: &str) -> Result<(), CompileError> {
    let mut previous = None;
    let mut normal = false;
    for component in Path::new(path).components() {
        match component {
            Component::Normal(name) => {
                if previous == Some(name) {
                    return Err(CompileError(format!(
                        "repeated module path component: {path}"
                    )));
                }
                previous = Some(name);
                normal = true;
            }
            Component::CurDir => {}
            _ => {
                return Err(CompileError(format!(
                    "module path must be relative and contain no '..': {path}"
                )));
            }
        }
    }
    if !normal {
        return Err(CompileError("module path must not be empty".into()));
    }
    Ok(())
}
fn expand_search(path: &Path, base: &Path) -> Result<Option<PathBuf>, CompileError> {
    let text = path.to_string_lossy();
    let expanded = if text == "~" || text.starts_with("~/") {
        let Some(home_dir) = std::env::var_os("HOME") else {
            return Ok(None);
        };
        PathBuf::from(home_dir).join(text.strip_prefix("~/").unwrap_or(""))
    } else if text == "$ORIGIN" || text.starts_with("$ORIGIN/") {
        let executable =
            std::env::current_exe().map_err(|e| CompileError(format!("$ORIGIN: {e}")))?;
        executable
            .parent()
            .ok_or_else(|| CompileError("$ORIGIN has no directory".into()))?
            .join(text.strip_prefix("$ORIGIN/").unwrap_or(""))
    } else {
        path.to_path_buf()
    };
    Ok(Some(if expanded.is_absolute() {
        expanded
    } else {
        base.join(expanded)
    }))
}
fn import_error(source: &ParsedModule, import: &Import, error: CompileError) -> CompileError {
    let location = import
        .span
        .as_ref()
        .and_then(|s| source.files.locate(s.start));
    let line = location.map_or(1, |l| l.line);
    CompileError(format!(
        "{}:{line}: importing {:?}: {error}",
        source.path.display(),
        import.path
    ))
}
