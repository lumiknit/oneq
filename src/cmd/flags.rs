use crate::render;
use clap::{Parser, Subcommand};
use std::io::IsTerminal;

#[derive(Parser, Debug)]
pub struct ReplArgs {
    #[arg(long = "ps1", env = "1Q_PS1", help = "prompt string for the REPL")]
    pub ps1: Option<String>,

    #[arg(
        long = "ps2",
        env = "1Q_PS2",
        help = "prompt string for the REPL continuation line"
    )]
    pub ps2: Option<String>,

    #[arg(trailing_var_arg = true, help = "f")]
    pub files: Vec<String>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    #[command(
        name = "--build-configuration",
        about = "show jq's build configuration",action = ArgAction::SetTrue
    )]
    BuildConfiguration,

    #[command(name = "--lsp", about = "(UNIMPLEMENTED) run jq as a language server")]
    LSP,

    #[command(
        name = "--repl",
        about = "repl mode",
        action = ArgAction::SetTrue,
    )]
    Repl(ReplArgs),
}

#[derive(Parser, Debug)]
#[command(
    disable_help_subcommand = true,
    version,
    about = "1q - extended jq",
    long_about = r#"
    1q - extended jq

    Tips:
    - For input files (positional arguments) and '--from-file' arguments, the input files are chained together as a single input stream, and the filter is applied to each input value in turn.
    - In in-place mode, multiple files does not be chained. Filter applied one-by-one.
    - '--doc' option put the jq reference in JSON as input value.
    - Some output options are ignored for non-JSON output formats.
    - Most formats strictly parse the input. For loose parsing, use json5 or j (loose-json)
    "#
)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Commands>,

    #[arg(
        long = "dump-ir",
        help = "compile the filter and print its final resolved Expr"
    )]
    pub dump_ir: bool,

    #[arg(
        long,
        global = true,
        help = "input and output format will be 'jq', and set filter as '.'"
    )]
    pub fmt: bool,

    // 1q options
    #[arg(
        short = 'F',
        long = "from",
        help_heading = "Data Format",
        help = "(1q) specify input format (json, json5, yaml, toml, xml, csv, tsv)"
    )]
    pub in_fmt: Option<String>,

    #[arg(
        short = 'T',
        long = "to",
        help_heading = "Data Format",
        help = "(1q) specify output format (json, json5, yaml, toml, xml, csv, tsv)"
    )]
    pub out_fmt: Option<String>,

    #[arg(
        long = "inline-output",
        global = true,
        help_heading = "Output Style",
        help = "(1q) single line similar to compact-output, more readable"
    )]
    pub inline_output: bool,

    #[arg(
        short = 'q',
        long = "quiet",
        help_heading = "Output Options",
        help = "(1q) do not print anything"
    )]
    pub quiet: bool,

    #[arg(
        long = "doc",
        help_heading = "Input Options",
        help = "use jq reference sheet as the single input value"
    )]
    pub doc_input: bool,

    #[arg(short = 'i', long = "in-place", help = "turn on in-place editing")]
    pub in_place: bool,

    // jq-compatible options
    #[arg(
        short = 'n',
        long = "null-input",
        help_heading = "Input Options",
        help = "use `null` as the single input value"
    )]
    pub null_input: bool,

    #[arg(
        short = 'R',
        long = "raw-input",
        help_heading = "Input Options",
        help = "read each line as string instead of JSON"
    )]
    pub raw_input: bool,

    #[arg(
        short = 's',
        long = "slurp",
        help_heading = "Input Options",
        help = "read all inputs into an array and use it as the single input value"
    )]
    pub slurp: bool,

    #[arg(
        short = 'c',
        long = "compact-output",
        global = true,
        help_heading = "Output Style",
        help = "compact instead of pretty-printed output"
    )]
    pub compact_output: bool,

    #[arg(
        short = 'r',
        long = "raw-output",
        help_heading = "Output Options",
        help = "output strings without escapes and quotes"
    )]
    pub raw_output: bool,

    #[arg(
        long = "raw-output0",
        help_heading = "Output Options",
        help = "implies -r and output NUL after each output"
    )]
    pub raw_output0: bool,

    #[arg(
        short = 'j',
        long = "join-output",
        help_heading = "Output Options",
        help = "implies -r and output without newline after each output"
    )]
    pub join_output: bool,

    #[arg(
        short = 'a',
        long = "ascii-output",
        global = true,
        help_heading = "Output Options",
        help = "output strings by only ASCII characters using escape sequences"
    )]
    pub ascii_output: bool,

    #[arg(
        short = 'S',
        long = "sort-keys",
        help_heading = "Output Options",
        help = "sort keys of each object on output"
    )]
    pub sort_keys: bool,

    #[arg(
        short = 'C',
        long = "color-output",
        global = true,
        help_heading = "Output Style",
        help = "colorize JSON output"
    )]
    pub color_output: bool,

    #[arg(
        short = 'M',
        long = "monochrome-output",
        global = true,
        help_heading = "Output Style",
        help = "disable colored output"
    )]
    pub monochrome_output: bool,

    #[arg(
        long = "tab",
        global = true,
        help_heading = "Output Style",
        help = "use tabs for indentation"
    )]
    pub tab: bool,

    #[arg(
        long = "indent",
        value_name = "n",
        global = true,
        help_heading = "Output Style",
        help = "use n spaces for indentation (max 7 spaces)"
    )]
    pub indent: Option<u8>,

    #[arg(
        long = "unbuffered",
        help_heading = "Output Options",
        help = "flush output stream after each output"
    )]
    pub unbuffered: bool,

    #[arg(
        long = "stream",
        help_heading = "Output Options",
        help = "parse the input value in streaming fashion"
    )]
    pub stream: bool,

    #[arg(
        long = "stream-errors",
        help_heading = "Output Options",
        help = "implies --stream and report parse error as an array"
    )]
    pub stream_errors: bool,

    #[arg(
        long = "seq",
        help_heading = "Output Options",
        help = "parse input/output as application/json-seq"
    )]
    pub seq: bool,

    #[arg(
        short = 'f',
        long = "from-file",
        value_name = "file",
        help = "load filter from the file"
    )]
    pub from_file: Option<String>,

    #[arg(
        short = 'L',
        value_name = "directory",
        help = "search modules from the directory"
    )]
    pub module_dir: Option<String>,

    // Named and positional arguments
    #[arg(
        long = "arg",
        number_of_values = 2,
        value_names = ["name", "value"],
        global = true,
        help_heading = "Engine Arguments",
        help = "set $name to the string value")]
    pub arg: Vec<String>,

    #[arg(long = "argjson", number_of_values = 2, value_names = ["name", "value"], global = true, help_heading = "Engine Arguments",help = "set $name to the JSON value")]
    pub argjson: Vec<String>,

    #[arg(long = "slurpfile", number_of_values = 2, value_names = ["name", "file"], global=true, help_heading = "Engine Arguments",help = "set $name to an array of JSON values read from the file")]
    pub slurpfile: Vec<String>,

    #[arg(long = "rawfile", number_of_values = 2, value_names = ["name", "file"], global=true, help_heading = "Engine Arguments",help = "set $name to string contents of file")]
    pub rawfile: Vec<String>,

    #[arg(
        long = "args",
        global = true,
        help_heading = "Engine Arguments",
        help = "consume arguments as positional string values"
    )]
    pub args: bool,

    #[arg(
        long = "jsonargs",
        global = true,
        help_heading = "Engine Arguments",
        help = "consume arguments as positional JSON values"
    )]
    pub jsonargs: bool,

    #[arg(
        short = 'e',
        long = "exit-status",
        help = "set exit status code based on the output"
    )]
    pub exit_status: bool,

    // Positional arguments
    #[arg(
        index = 1,
        allow_hyphen_values = true,
        help = "inline jq script, default is '.'"
    )]
    pub filter: Option<String>,

    #[arg(index = 2, num_args = 0.., trailing_var_arg = true,
    	help = "input files default, positional arguments if --args or --jsonargs is specified")]
    pub rest: Vec<String>,

    // Internal fields
    #[clap(skip)]
    pub in_bytes: Option<Vec<u8>>,
}

impl Args {
    /// Returns the output compact level based on the flags set. The levels are defined as follows:
    /// - 0: Pretty-printed output (Default)
    /// - 1: Inline output (single line, more readable)
    /// - 2: Compact output (no extra whitespace)
    #[must_use]
    pub const fn output_compact_level(&self) -> render::CompactLevel {
        if self.compact_output {
            render::CompactLevel::Compact
        } else if self.inline_output {
            render::CompactLevel::Inline
        } else {
            render::CompactLevel::Pretty
        }
    }

    #[must_use]
    pub fn color_output(&self) -> bool {
        self.color_output_for(false)
    }

    #[must_use]
    pub fn color_output_for(&self, file_output: bool) -> bool {
        if self.monochrome_output {
            return false;
        }
        if self.color_output {
            return true;
        }
        !file_output
            && !cfg!(all(target_arch = "wasm32", target_os = "unknown"))
            && std::io::stdout().is_terminal()
    }

    #[must_use]
    pub fn build_output_style(&self) -> render::FormatOptions {
        let mut style = render::FormatOptions::default();
        style
            .with_compact_level(self.output_compact_level())
            .with_indent(self.tab, self.indent.unwrap_or(2));

        if self.quiet {
            style.with_quiet();
        }
        if self.ascii_output {
            style.with_ascii_output();
        }
        if self.sort_keys {
            style.with_sort_keys();
        }
        if self.unbuffered {
            style.with_unbuffered();
        }

        // Detailed styles

        style.raw = self.raw_output || self.raw_output0 || self.join_output;
        style.doc_end = if self.raw_output0 {
            Some("\0")
        } else if self.join_output {
            Some("")
        } else {
            Some("\n")
        };
        style.doc_begin = if self.seq { Some("\x1E") } else { None };
        style.doc_end_flush = self.unbuffered;
        style
    }
}
