//! Lexical names are separate from stable declaration identities and frame slots.
use super::{ir::*, parser::pairs::SpanPos};
use crate::strs::Symbol;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct Binding {
    pub name: Symbol,
    pub declaration: Option<SpanPos>,
    pub module: ModuleId,
}

#[derive(Clone, Debug, Default)]
pub struct Names {
    pub(crate) bindings: HashMap<Symbol, BindingId>,
    pub functions: HashMap<(Symbol, usize), CallTarget>,
    pub(crate) labels: HashMap<Symbol, LabelId>,
}

#[derive(Clone, Debug, Default)]
pub struct Symbols {
    pub(crate) names: Names,
    pub(crate) bindings: Vec<Binding>,
    pub(crate) next_function: usize,
    pub(crate) next_label: usize,
}

impl Symbols {
    /// Compiler-generated binding; it must not participate in name lookup.
    pub(crate) fn fresh_binding(&mut self, declaration: Option<SpanPos>) -> BindingId {
        let id = BindingId(self.bindings.len());
        self.bindings.push(Binding {
            name: crate::strs::keyword_temporary(),
            declaration,
            module: ModuleId(0),
        });
        id
    }

    pub(crate) fn declare(&mut self, name: Symbol, declaration: Option<SpanPos>) -> BindingId {
        let id = BindingId(self.bindings.len());
        self.bindings.push(Binding {
            name,
            declaration,
            module: ModuleId(0),
        });
        self.names.bindings.insert(name, id);
        id
    }

    pub(crate) fn lookup(&self, name: Symbol) -> Option<BindingId> {
        self.names.bindings.get(&name).copied()
    }
}
