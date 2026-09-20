//! CLI input/output policy around the same Session used by the REPL.
use crate::{
    cmd::flags,
    data::{self, Value},
    io::SharedInputTracker,
    jq::{
        CompileOptions, Session,
        vm::{InputMode, JqError, VmEvent, host::Host},
    },
    strs::Symbol,
};
use std::io::Write;
mod helper;
pub(crate) mod option;
use option::RunOption;

struct CliHost<I> {
    inputs: I,
    tracker: SharedInputTracker,
    module_dirs: Vec<std::path::PathBuf>,
    filter_path: String,
}

impl<I: Iterator<Item = Result<Value, data::DataError>>> Host for CliHost<I> {
    fn next_input(&mut self) -> Option<Result<Value, JqError>> {
        self.inputs
            .next()
            .map(|r| r.map_err(|e| JqError::Input(e.to_string())))
    }

    fn environment(&mut self) -> Result<Value, JqError> {
        Ok(helper::environment())
    }

    fn input_filename(&self) -> Option<Symbol> {
        self.tracker.borrow().filename
    }

    fn input_line_number(&self) -> Option<usize> {
        Some(self.tracker.borrow().line_number)
    }

    fn modulemeta(&mut self, name: &Value) -> Result<Value, JqError> {
        let Value::String(name) = name else {
            return Err(JqError::Runtime(Value::String(
                "modulemeta requires a string module name"
                    .to_string()
                    .into(),
            )));
        };
        helper::modulemeta(name, &self.filter_path, &self.module_dirs)
            .map_err(|e| JqError::Runtime(Value::String(e.to_string().into())))
    }
}

pub fn run(args: &flags::Args) {
    match safe_run(args) {
        Ok(status) => std::process::exit(status),
        Err(error) => {
            eprintln!("1q: error: {error}");
            std::process::exit(1);
        }
    }
}

pub(crate) fn safe_run(args: &flags::Args) -> anyhow::Result<i32> {
    let opt = RunOption::from_args(args)?;
    let mut session = Session::new();
    let mut names: Vec<_> = opt.globals.keys().collect();
    names.sort();
    for name in names {
        session.bind(name, opt.globals[name].clone());
    }
    // Compile once before opening or consuming input.
    let entry = match session.append(
        &opt.filter,
        CompileOptions {
            path: opt.filter_path.clone(),
            module_dirs: opt.module_dirs.clone(),
            ..CompileOptions::default()
        },
    ) {
        Ok(entry) => entry,
        Err(error) => {
            writeln!(crate::io::stderr(), "1q: compile error: {error}")?;
            return Ok(3);
        }
    };
    if opt.dump_ir {
        print!(
            "{}",
            session
                .dump_expr(entry)
                .map_err(|e| anyhow::anyhow!(e.user_message()))?
        );
        return Ok(0);
    }

    let (parser, tracker) = opt.build_parser()?;
    let builder = data::ValueBuilder::new(parser, opt.stream);
    let mut host = CliHost {
        inputs: data::ValueCollector::new(builder, opt.slurp),
        tracker,
        module_dirs: opt.module_dirs.clone(),
        filter_path: opt.filter_path.clone(),
    };

    let mode = if opt.null_input {
        InputMode::Null
    } else {
        InputMode::Host
    };

    let mut execution = session
        .run(entry, &mut host, mode)
        .map_err(|e| anyhow::anyhow!(e.user_message()))?;
    execution.continue_after_error();

    let mut serializer = opt.build_serializer()?;
    loop {
        match execution.resume_mode::<false>(0) {
            VmEvent::Output(value) => {
                serializer.put(value)?;
            }
            VmEvent::Suspended => unreachable!(),
            VmEvent::Done => break,
            VmEvent::Error(JqError::Input(error)) => {
                writeln!(crate::io::stderr(), "1q: {error}")?;
                return Ok(4);
            }
            VmEvent::Error(error) => {
                writeln!(crate::io::stderr(), "1q: error: {}", error.user_message())?;
            }
            VmEvent::Halt { code, value } => {
                match value {
                    Value::Null => {}
                    Value::String(s) => crate::io::stderr().write_all(s.as_bytes())?,
                    value => writeln!(crate::io::stderr(), "{value}")?,
                }
                return Ok(code);
            }
        }
    }

    let (failed, last) = execution.input_status();
    Ok(if failed {
        5
    } else if opt.exit_status {
        match last {
            Some(true) => 0,
            Some(false) => 1,
            None => 4,
        }
    } else {
        0
    })
}
