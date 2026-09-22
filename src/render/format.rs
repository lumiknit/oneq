const EMPTY: &str = "";
const RS: &str = "\x1e";
const TAB: &str = "\t";
const NUL: &str = "\0";
const SPACES: &str = "        "; // Greater than 7 spaces is not supported by `with_indent` method.

#[derive(Default, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum CompactLevel {
    #[default]
    /// pretty print with indent.
    /// Most formats' default output style is pretty print.
    Pretty = 0,

    /// put everything in a single line as much as possible.
    /// However, unlike compact, it'll keep some whitespace to make the output more readable.
    /// For example: {"a": 24, "b": [3, 4, 5]}
    /// It'll delete all comments and extra whitespace.
    Inline = 1,

    /// put everything in a single line without extra whitespace.
    /// For example: {"a":24,"b":[3,4,5]}
    Compact = 2,
}

#[derive(Default, Clone, Copy)]
pub struct FormatOptions {
    /// if quiet, no output should be printed.
    pub quiet: bool,

    /// `compact_level` represents the compact level of output style.
    pub compact_level: CompactLevel,

    /// raw denotes pure string output should be print as is.
    /// All other types follows each format's default output style.
    pub raw: bool,

    /// indent represents the indent style.
    /// For pretty print, repeat print this string for each level of indentation.
    pub indent: &'static str,

    /// Preferred maximum visual line width for recursive pretty printers.
    /// Zero selects the jq default of 79 columns.
    pub max_width: usize,

    /// `ascii_only` denotes the output should be ASCII only.
    /// If the format has escape notation for non-ASCII characters, it'll aggressively use it
    /// to make sure the output is ASCII only.
    /// Some format (e.g. CSV) doesn't have the notation and this option may be ignored.
    pub ascii_only: bool,

    /// `doc_begin` will be print just before every document begins.
    /// In most case this is not used, but for jq `--seq` option,
    /// It'll need to print `\x1e` before every document begins. (See json-seq)
    pub doc_begin: Option<&'static str>,

    /// `doc_end` will be print just after every document ends.
    /// If 'none', it'll print default document separator for each format.
    /// e.g. json: '\n', csv: '\n\n', yaml: '\n---\n', etc.
    /// Some option in jq may change this behavior to other things, for example:
    /// - --raw-output: Empty string
    /// - --raw-output0: '\0'
    /// - --join-output: Empty
    pub doc_end: Option<&'static str>,

    /// `doc_end_flush` is true if flush requires for every document ends.
    /// In 'jq', '--unbuffered' option will set this true.
    pub doc_end_flush: bool,

    /// `sort_keys` is true if the output should sort keys of each object.
    pub sort_keys: bool,
}

impl FormatOptions {
    #[must_use]
    pub const fn max_width(&self) -> usize {
        if self.max_width == 0 {
            79
        } else {
            self.max_width
        }
    }

    pub const fn with_max_width(&mut self, width: usize) -> &mut Self {
        self.max_width = width;
        self
    }
    pub const fn with_compact_level(&mut self, level: CompactLevel) -> &mut Self {
        self.compact_level = level;
        self
    }

    pub fn with_indent(&mut self, tab: bool, spaces: u8) -> &mut Self {
        if tab {
            self.indent = TAB;
        } else {
            let n = spaces.min(7) as usize;
            self.indent = &SPACES[0..n];
        }
        self
    }

    pub const fn with_quiet(&mut self) -> &mut Self {
        self.quiet = true;
        self
    }

    pub const fn with_no_doc_end(&mut self) -> &mut Self {
        self.doc_end = Some(EMPTY);
        self
    }

    pub const fn with_doc_end(&mut self, s: &'static str) -> &mut Self {
        self.doc_end = Some(s);
        self
    }

    pub const fn with_raw_output(&mut self) -> &mut Self {
        self.raw = true;
        self.doc_end = Some(EMPTY);
        self
    }

    pub const fn with_raw_output0(&mut self) -> &mut Self {
        self.raw = true;
        self.doc_end = Some(NUL);
        self
    }

    pub const fn with_join_output(&mut self) -> &mut Self {
        self.raw = true;
        self.doc_end = Some(EMPTY);
        self
    }

    pub const fn with_ascii_output(&mut self) -> &mut Self {
        self.ascii_only = true;
        self
    }

    pub const fn with_sort_keys(&mut self) -> &mut Self {
        self.sort_keys = true;
        self
    }

    pub const fn with_seq(&mut self) -> &mut Self {
        self.doc_begin = Some(RS);
        self
    }

    pub const fn with_unbuffered(&mut self) -> &mut Self {
        self.doc_end_flush = true;
        self
    }
}
