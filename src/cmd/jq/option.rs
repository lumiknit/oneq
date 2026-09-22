use std::{collections::HashMap, path::PathBuf, rc::Rc, str::FromStr};

use crate::cmd::flags;
use crate::data::builder::StreamOption;
use crate::{
    data::{self, DataFormat, Value},
    doc,
    io::{self, Input, Output},
    render, strs,
};

#[derive(Clone)]
pub(super) enum RunInputSource {
    Stdin,
    Files(Vec<String>),
    String(String),
    Bytes(Vec<u8>),
}

#[derive(Clone)]
pub struct RunOption {
    pub dump_ir: bool,
    pub from_fmt: DataFormat,
    pub to_fmt: DataFormat,

    pub render_options: render::Options,

    pub stream: StreamOption,
    pub slurp: bool,

    pub filter: String,
    pub filter_path: String,
    pub module_dirs: Vec<PathBuf>,

    pub globals: HashMap<String, Value>,
    pub null_input: bool,
    pub exit_status: bool,
    pub in_place: bool,
    pub files: Vec<String>,
    source: RunInputSource,
}

impl RunOption {
    pub fn from_args(args: &flags::Args) -> anyhow::Result<Self> {
        // Formats
        let mut from_format = args
            .in_fmt
            .as_deref()
            .map(DataFormat::from_str)
            .transpose()
            .map_err(|()| anyhow::anyhow!("unknown input format"))?
            .unwrap_or_default();
        let mut to_format = args
            .out_fmt
            .as_deref()
            .map(DataFormat::from_str)
            .transpose()
            .map_err(|()| anyhow::anyhow!("unknown output format"))?
            .unwrap_or_default();

        let mut slurp = args.slurp;

        if args.raw_input {
            if slurp {
                from_format = DataFormat::RawSlurp;
                slurp = false;
            } else {
                from_format = DataFormat::Raw;
            }
        }
        if args.raw_output {
            to_format = DataFormat::Raw;
        }
        if args.raw_output0 {
            to_format = DataFormat::Raw;
        }
        if args.join_output {
            to_format = DataFormat::Raw;
        }
        if args.seq {
            // JSON-seq
            from_format = DataFormat::Json;
            to_format = DataFormat::Json;
        }
        if args.fmt {
            // If format mode is on, use filter to '.'.
            from_format = DataFormat::JQ;
            to_format = DataFormat::JQ;
        }

        // Render options
        let mut render_options = render::Options::new(args.build_output_style());
        if args.color_output_for(args.in_place) {
            render_options.with_theme(render::ColorTheme::default());
        }

        let filter: String;
        let mut files: Vec<String>;

        if args.fmt {
            // If fmt is on, filter must be consider as '.'
            filter = ".".to_string();
            files = vec![];
            if let Some(f) = &args.filter {
                // An explicit identity filter still formats stdin.
                if f != "." {
                    files.push(f.clone());
                }
            }
            files.extend(args.rest.clone());
        } else if let Some(path) = &args.from_file {
            // Read filter first
            filter = std::fs::read_to_string(path)?;
            files = vec![];
            if let Some(f) = &args.filter {
                // First positional argument, just push
                files.push(f.clone());
            }
            files.extend(args.rest.clone());
        } else {
            filter = args.filter.clone().unwrap_or_else(|| ".".to_string());
            files = args.rest.clone();
        }

        let globals = arguments(args)?;
        if args.args || args.jsonargs {
            files.clear();
        }

        let source: RunInputSource;
        if args.doc_input && !args.null_input {
            from_format = DataFormat::Json;
            source = RunInputSource::String(String::from_utf8(doc::json()).unwrap());
        } else if let Some(v) = &args.in_bytes {
            source = RunInputSource::Bytes(v.clone());
        } else if !files.is_empty() {
            source = RunInputSource::Files(files.clone());
        } else {
            source = RunInputSource::Stdin;
        }

        let stream: StreamOption = if args.stream_errors {
            StreamOption::StreamError
        } else if args.stream {
            StreamOption::Stream
        } else {
            StreamOption::default()
        };

        Ok(Self {
            dump_ir: args.dump_ir,
            from_fmt: from_format,
            to_fmt: to_format,
            render_options,
            stream,
            slurp,
            filter,
            filter_path: args
                .from_file
                .clone()
                .unwrap_or_else(|| "<command-line>".into()),
            module_dirs: args.module_dir.iter().map(PathBuf::from).collect(),
            globals,
            null_input: args.null_input,
            exit_status: args.exit_status,
            in_place: args.in_place,
            files: files.clone(),
            source,
        })
    }

    pub fn build_parser(
        &self,
    ) -> anyhow::Result<(data::AnyParser<'static>, io::SharedInputTracker)> {
        self.build_parser_for(&self.source)
    }

    pub(super) fn build_parser_for(
        &self,
        source: &RunInputSource,
    ) -> anyhow::Result<(data::AnyParser<'static>, io::SharedInputTracker)> {
        let tracker = io::InputTracker::shared();
        let i = match source {
            RunInputSource::Stdin => Input::new_stdin_with_tracker(tracker.clone()),
            RunInputSource::Files(files) => {
                let refs: Vec<&str> = files.iter().map(std::string::String::as_str).collect();
                Input::new_files_with_tracker(refs.as_slice(), tracker.clone())?
            }
            RunInputSource::String(s) => Input::new_string_with_tracker(s.clone(), tracker.clone()),
            RunInputSource::Bytes(s) => Input::new_bytes_with_tracker(s.clone(), tracker.clone()),
        };
        let parser = data::AnyParser::new(self.from_fmt, i)
            .map_err(|e| anyhow::anyhow!("Failed to create parser: {e}"))?;
        Ok((parser, tracker))
    }

    pub fn build_serializer_with_output(
        &self,
        output: Output,
    ) -> anyhow::Result<data::AnySerializer> {
        data::AnySerializer::from_format(self.to_fmt, output, self.render_options.clone())
            .map_err(|e| anyhow::anyhow!("Failed to create serializer: {e}"))
    }
}

fn json_values(source: String) -> anyhow::Result<Vec<Value>> {
    let parser = data::json_parser(Input::new_str(source.as_str()));
    Ok(data::ValueBuilder::new(parser, StreamOption::default()).collect::<Result<Vec<_>, _>>()?)
}
fn json_argument(source: &str) -> anyhow::Result<Value> {
    let mut values = json_values(source.to_owned())?;
    anyhow::ensure!(
        values.len() == 1,
        "argument must contain exactly one JSON value"
    );
    Ok(values.remove(0))
}
fn arguments(args: &flags::Args) -> anyhow::Result<HashMap<String, Value>> {
    let mut named = indexmap::IndexMap::new();
    for kv in args.arg.as_chunks::<2>().0 {
        named.insert(strs::intern(&kv[0]), Value::String(kv[1].clone().into()));
    }
    for kv in args.argjson.as_chunks::<2>().0 {
        named.insert(strs::intern(&kv[0]), json_argument(&kv[1])?);
    }
    for kv in args.rawfile.as_chunks::<2>().0 {
        named.insert(
            strs::intern(&kv[0]),
            Value::String(std::fs::read_to_string(&kv[1])?.into()),
        );
    }
    for kv in args.slurpfile.as_chunks::<2>().0 {
        named.insert(
            strs::intern(&kv[0]),
            Value::Array(Rc::new(json_values(std::fs::read_to_string(&kv[1])?)?)),
        );
    }
    let mut positional = vec![];
    if args.args || args.jsonargs {
        let values = args
            .from_file
            .as_ref()
            .and(args.filter.as_ref())
            .into_iter()
            .chain(args.rest.iter());
        for v in values {
            positional.push(if args.jsonargs {
                json_argument(v)?
            } else {
                Value::String(v.clone().into())
            });
        }
    }
    let mut globals = HashMap::from([("ENV".into(), super::helper::environment())]);
    for (k, v) in &named {
        globals.insert(strs::resolve(*k).unwrap().into(), v.clone());
    }
    globals.insert(
        "ARGS".into(),
        Value::Object(Rc::new(indexmap::IndexMap::from([
            (strs::keyword_named(), Value::Object(Rc::new(named))),
            (
                strs::keyword_positional(),
                Value::Array(Rc::new(positional)),
            ),
        ]))),
    );
    Ok(globals)
}
