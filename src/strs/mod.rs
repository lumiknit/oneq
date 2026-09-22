//! strs is a simple string pool

use std::{collections::HashMap, sync::RwLock};

pub type Symbol = isize;

static STRING_POOL: RwLock<Option<HashMap<&'static str, Symbol>>> = RwLock::new(None);
static STRING_LIST: RwLock<Vec<&'static str>> = RwLock::new(Vec::new());

macro_rules! keyword_symbols {
    (@count $($name:ident),*) => {
        0usize $(+ { let _ = stringify!($name); 1usize })*
    };
    (@define [$($previous:ident),*] $name:ident = $value:literal) => {
        #[inline]
        pub const fn $name() -> Symbol {
            keyword_symbols!(@count $($previous),*) as Symbol
        }
    };
    (@define [$($previous:ident),*] $name:ident = $value:literal $(, $rest_name:ident = $rest_value:literal)* $(,)?) => {
        #[inline]
        pub const fn $name() -> Symbol {
            keyword_symbols!(@count $($previous),*) as Symbol
        }
        keyword_symbols!(@define [$($previous,)* $name] $($rest_name = $rest_value),*);
    };
    ($( $name:ident = $value:literal ),* $(,)?) => {
        keyword_symbols!(@define [] $($name = $value),*);
        const KEYWORDS: &[&str] = &[$($value),*];
    };
}

keyword_symbols! {
    keyword_as = "as",
    keyword_defs = "defs",
    keyword_deps = "deps",
    keyword_is_data = "is_data",
    keyword_relpath = "relpath",
    keyword_named = "named",
    keyword_positional = "positional",
    keyword_underscore = "_",
    keyword_plus = "+",
    keyword_env = "env",
    keyword_file = "file",
    keyword_line = "line",
    keyword_tostring = "tostring",
    keyword_search = "search",
    keyword_children = "children",
    keyword_content = "content",
    keyword_tag = "tag",
    keyword_temporary = "<temporary>",
    keyword_start = "start",
    keyword_end = "end",
}

fn initialize_keywords() {
    let mut pool = STRING_POOL.write().unwrap();
    if pool.is_some() {
        return;
    }
    let mut map = HashMap::with_capacity(KEYWORDS.len());
    {
        let mut list = STRING_LIST.write().unwrap();
        for (index, keyword) in KEYWORDS.iter().enumerate() {
            map.insert(*keyword, index as Symbol);
            list.push(keyword);
        }
    }
    *pool = Some(map);
}

/// Tries to intern a string into a symbol, returning None if the string is not interned.
pub fn try_intern(s: &str) -> Option<Symbol> {
    initialize_keywords();
    if let Some(pool) = &*STRING_POOL.read().unwrap()
        && let Some(&symbol) = pool.get(s)
    {
        return Some(symbol);
    }
    None
}

/// Interns a string into a symbol.
pub fn intern(s: &str) -> Symbol {
    initialize_keywords();
    if let Some(s) = try_intern(s) {
        return s;
    }

    // If not, intern it
    let mut pool = STRING_POOL.write().unwrap();

    // After acquiring the write lock, check again if the string is already interned during the time we were waiting for the lock
    if let Some(pool) = &*pool
        && let Some(&symbol) = pool.get(s)
    {
        return symbol;
    }

    let mut string_list = STRING_LIST.write().unwrap();

    // Intern the string
    assert!(
        string_list.len() < i64::MAX as usize,
        "String pool overflow"
    );

    let symbol = string_list.len() as Symbol;
    let static_str: &'static str = Box::leak(s.to_string().into_boxed_str());
    string_list.push(static_str);
    drop(string_list); // Release the lock on STRING_LIST before acquiring the lock on STRING_POOL

    pool.get_or_insert_with(HashMap::new)
        .insert(static_str, symbol);
    symbol
}

pub fn resolve(symbol: Symbol) -> Option<&'static str> {
    initialize_keywords();
    let string_list = STRING_LIST.read().unwrap();
    string_list.get(symbol as usize).copied()
}
