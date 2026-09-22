//! WASM entrypoint. Packets contain JSON argv, a NUL separator, then stdin.
use crate::strs::Symbol;
use crate::{
    cmd,
    data::{self, Value},
    io::SharedInputTracker,
    jq::{
        CompileOptions, Session,
        vm::{InputMode, JqError, VmEvent, host::Host, session::Execution},
    },
};
use clap::Parser;
use std::{
    cell::RefCell,
    io::{self, Write},
    rc::Rc,
};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(module = "/web/wasm-output.js")]
extern "C" {
    #[wasm_bindgen(catch)]
    fn write(channel: u8, bytes: &[u8]) -> Result<(), JsValue>;
}

pub(crate) struct Stream(pub u8);
impl Write for Stream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        write(self.0, bytes).map_err(|_| io::Error::other("WASM output callback failed"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct WasmHost<I> {
    inputs: I,
    tracker: SharedInputTracker,
}
impl<I: Iterator<Item = Result<Value, data::DataError>>> Host for WasmHost<I> {
    fn next_input(&mut self) -> Option<Result<Value, JqError>> {
        self.inputs
            .next()
            .map(|r| r.map_err(|e| JqError::Input(e.to_string())))
    }
    fn environment(&mut self) -> Result<Value, JqError> {
        Ok(Value::Object(Rc::new(Default::default())))
    }
    fn input_filename(&self) -> Option<Symbol> {
        self.tracker.borrow().filename
    }
    fn input_line_number(&self) -> Option<usize> {
        Some(self.tracker.borrow().line_number)
    }
}

struct WasmRun {
    execution: Execution<'static>,
    serializer: data::AnySerializer,
}
thread_local! { static RUN: RefCell<Option<WasmRun>> = const { RefCell::new(None) }; }

const SUSPENDED: i32 = 0;
const OUTPUT: i32 = 1;
const DONE: i32 = 2;
const ERROR: i32 = 3;
const HALTED: i32 = 4;

#[wasm_bindgen]
pub fn prepare(packet: &[u8]) -> i32 {
    match prepare_inner(packet) {
        Ok(()) => 0,
        Err(error) => {
            let _ = writeln!(Stream(2), "1q: error: {error}");
            ERROR
        }
    }
}

fn prepare_inner(packet: &[u8]) -> anyhow::Result<()> {
    let boundary = packet
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| anyhow::anyhow!("missing NUL separator after JSON argv"))?;
    let Value::Array(values) =
        crate::data::parse_json_str(std::str::from_utf8(&packet[..boundary])?)
            .map_err(|e| anyhow::anyhow!("invalid JSON argv: {e}"))?
    else {
        anyhow::bail!("argv must be a JSON array of strings")
    };
    let mut argv = vec!["1q".to_string()];
    for value in values.iter() {
        let Value::String(value) = value else {
            anyhow::bail!("argv must be a JSON array of strings")
        };
        anyhow::ensure!(!value.contains('\0'), "argv strings must not contain NUL");
        argv.push(value.to_string());
    }
    let mut args = match cmd::flags::Args::try_parse_from(argv) {
        Ok(args) => args,
        Err(error) => {
            let channel = if error.use_stderr() { 2 } else { 1 };
            write!(Stream(channel), "{}", error.render())?;
            return Err(anyhow::anyhow!("argument parsing failed"));
        }
    };
    validate(&args)?;
    args.in_bytes = Some(packet[boundary + 1..].to_vec());
    let opt = cmd::jq::option::RunOption::from_args(&args)?;
    let session = Box::leak(Box::new(Session::new()));
    let mut names: Vec<_> = opt.globals.keys().collect();
    names.sort();
    for name in names {
        session.bind(name, opt.globals[name].clone());
    }
    let entry = session
        .append(
            &opt.filter,
            CompileOptions {
                path: opt.filter_path.clone(),
                module_dirs: opt.module_dirs.clone(),
                ..Default::default()
            },
        )
        .map_err(|e| anyhow::anyhow!("compile error: {e}"))?;
    let (parser, tracker) = opt.build_parser()?;
    let builder = data::ValueBuilder::new(parser, opt.stream);
    let host = Box::leak(Box::new(WasmHost {
        inputs: data::ValueCollector::new(builder, opt.slurp),
        tracker,
    }));
    let mode = if opt.null_input {
        InputMode::Null
    } else {
        InputMode::Host
    };
    // SAFETY: the worker owns this execution, and the borrowed values live until it ends.
    let mut execution: Execution<'static> = session
        .run(entry, host, mode)
        .map_err(|e| anyhow::anyhow!(e.user_message()))?;
    execution.continue_after_error();
    let serializer = opt.build_serializer_with_output(crate::io::Output::Stdout)?;
    RUN.with(|run| {
        *run.borrow_mut() = Some(WasmRun {
            execution,
            serializer,
        })
    });
    Ok(())
}

#[wasm_bindgen]
pub fn resume(fuel: usize) -> i32 {
    RUN.with(|run| {
        let mut slot = run.borrow_mut();
        let Some(run) = slot.as_mut() else {
            return DONE;
        };
        let event = run.execution.resume(fuel);
        let status = match event {
            VmEvent::Output(value) => match run.serializer.put(value) {
                Ok(()) => OUTPUT,
                Err(e) => {
                    let _ = writeln!(Stream(2), "1q: error: {e}");
                    ERROR
                }
            },
            VmEvent::Suspended => SUSPENDED,
            VmEvent::Done => DONE,
            VmEvent::Error(error) => {
                let _ = writeln!(Stream(2), "1q: error: {}", error.user_message());
                ERROR
            }
            VmEvent::Halt { code, value } => {
                if !matches!(value, Value::Null) {
                    let _ = writeln!(Stream(2), "{value}");
                }
                let _ = code;
                HALTED
            }
        };
        if matches!(status, DONE | ERROR | HALTED) {
            *slot = None;
        }
        status
    })
}

#[wasm_bindgen]
pub fn cancel() {
    RUN.with(|run| *run.borrow_mut() = None);
}

/// Run synchronously for compatibility with the original WASM API.
#[wasm_bindgen]
pub fn run(packet: &[u8]) -> i32 {
    match execute(packet) {
        Ok(status) => status,
        Err(error) => {
            let _ = writeln!(Stream(2), "1q: error: {error}");
            2
        }
    }
}

// Return the build configuration embedded in the package.
#[wasm_bindgen]
pub fn build_configuration() -> String {
    crate::BuildConfig.to_string()
}

fn execute(packet: &[u8]) -> anyhow::Result<i32> {
    let boundary = packet
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| anyhow::anyhow!("missing NUL separator after JSON argv"))?;
    let header = std::str::from_utf8(&packet[..boundary])?;
    let Value::Array(values) = crate::data::parse_json_str(header)
        .map_err(|e| anyhow::anyhow!("invalid JSON argv: {e}"))?
    else {
        anyhow::bail!("argv must be a JSON array of strings");
    };
    let mut argv = vec!["1q".to_string()];
    for value in values.iter() {
        let Value::String(value) = value else {
            anyhow::bail!("argv must be a JSON array of strings");
        };
        anyhow::ensure!(!value.contains('\0'), "argv strings must not contain NUL");
        argv.push(value.to_string());
    }
    let mut args = match cmd::flags::Args::try_parse_from(argv) {
        Ok(args) => args,
        Err(error) => {
            let channel = if error.use_stderr() { 2 } else { 1 };
            write!(Stream(channel), "{}", error.render())?;
            return Ok(error.exit_code());
        }
    };
    args.in_bytes = Some(packet[boundary + 1..].to_vec());
    validate(&args)?;
    match cmd::jq::safe_run(&args) {
        Ok(status) => Ok(status),
        Err(error) => {
            writeln!(Stream(2), "1q: error: {error}")?;
            Ok(1)
        }
    }
}

fn validate(args: &cmd::flags::Args) -> anyhow::Result<()> {
    anyhow::ensure!(args.cmd.is_none(), "subcommands are not supported in WASM");
    anyhow::ensure!(
        args.from_file.is_none()
            && args.module_dir.is_none()
            && args.rawfile.is_empty()
            && args.slurpfile.is_empty()
            && !args.in_place,
        "file options are not supported in WASM"
    );
    anyhow::ensure!(
        args.rest.is_empty() || args.args || args.jsonargs,
        "input files are not supported in WASM"
    );
    anyhow::ensure!(
        !args.fmt || args.filter.as_deref().is_none_or(|f| f == "."),
        "--fmt file arguments are not supported in WASM"
    );
    Ok(())
}
