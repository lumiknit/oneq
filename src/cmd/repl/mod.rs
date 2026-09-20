//! Interactive controller around the native Session/compiler/VM.
mod controller;
use crate::{
    cmd::flags,
    data::{self, DataFormat},
    io::Output,
    jq::parser,
    render,
};
use controller::Controller;
use rustyline::{DefaultEditor, error::ReadlineError};
use std::str::FromStr;

fn print_help() {
    println!(
        "1q-repl commands:
  :help, :h             Show this command list
  :doc [FILTER], :d     Query references in an independent session
  :load <PATH>, :l      Load a JSON stream from a file (:file/:f also work)
  :to <FORMAT>          Set output serializer (e.g. :to yaml)
  :dump [PATH]          Dump accumulated jq scripts to screen or a file
  :reset                Reset input to null and clear session state
  :exit, :quit, :q      Exit

Filters run on each current input. Successful output becomes the next input.
The native engine is under development; unsupported filters report an error."
    );
}
fn run_doc_command(filter: &str) {
    let bytes = miniz_oxide::inflate::decompress_to_vec_zlib(include_bytes!(concat!(
        env!("OUT_DIR"),
        "/doc.json.zz"
    )))
    .expect("failed to decompress doc.json");
    let doc = String::from_utf8(bytes).expect("doc.json is not valid UTF-8");
    if filter.is_empty() {
        print!("{doc}");
        return;
    }
    let mut controller = Controller::default();
    let result = controller
        .load(&doc, DataFormat::Json)
        .and_then(|_| controller.evaluate(filter).map(|values| values.to_vec()));
    match result {
        Ok(values) => {
            for value in values {
                println!("{value}");
            }
        }
        Err(error) => eprintln!("1q repl: {error}"),
    }
}
fn wrap_prompt(prompt: &Option<String>, color: bool, default: &str) -> (String, String) {
    let raw = prompt.clone().unwrap_or_else(|| default.into());
    let styled = if prompt.is_none() && color {
        format!(
            "{}0;1;31m{}{}",
            render::ANSI_STYLE_ESC,
            default,
            render::RESET
        )
    } else {
        raw.clone()
    };
    (raw, styled)
}
pub fn run(args: &flags::Args) {
    let Some(flags::Commands::Repl(options)) = &args.cmd else {
        return;
    };
    let mut controller = Controller::default();
    let mut render_options = render::Options::new(args.build_output_style());
    if args.color_output() {
        render_options.with_theme(render::ColorTheme::default());
    }
    let mut serializer = match data::AnySerializer::from_format(
        DataFormat::Json,
        Output::Stdout,
        render_options.clone(),
    ) {
        Ok(serializer) => serializer,
        Err(error) => {
            eprintln!("1q repl: {error}");
            return;
        }
    };
    if !options.files.is_empty() {
        let mut input = String::new();
        for file in &options.files {
            match std::fs::read_to_string(file) {
                Ok(text) => {
                    input.push_str(&text);
                    input.push('\n');
                }
                Err(error) => {
                    eprintln!("1q repl: {file}: {error}");
                    std::process::exit(1);
                }
            }
        }
        let format =
            data::formats::extension::from_extension(&options.files[0]).unwrap_or(DataFormat::Json);
        if let Err(error) = controller.load(&input, format) {
            eprintln!("1q repl: {error}");
            std::process::exit(1);
        }
    }
    // rustyline's Windows backend needs the unstyled prompt for layout and
    // editing, while the styled form is used only when drawing the prompt.
    let ps1 = wrap_prompt(&options.ps1, args.color_output(), "1q> ");
    let ps2 = wrap_prompt(&options.ps2, args.color_output(), "..> ");
    println!("Welcome to 1q REPL. Type :help for help, :q to exit");
    let mut rl = DefaultEditor::new().expect("failed to init line editor");
    let mut buffer = String::new();
    let mut exit_armed = false;
    loop {
        let line = match rl.readline(if buffer.is_empty() { &ps1 } else { &ps2 }) {
            Ok(line) => {
                exit_armed = false;
                line
            }
            Err(ReadlineError::Interrupted) => {
                if !buffer.is_empty() {
                    buffer.clear();
                    exit_armed = false;
                } else if exit_armed {
                    break;
                } else {
                    exit_armed = true;
                    eprintln!("(To exit, press Ctrl-C again or type :q)");
                }
                continue;
            }
            Err(ReadlineError::Eof) => break,
            Err(error) => {
                eprintln!("1q repl: {error}");
                break;
            }
        };
        if buffer.is_empty()
            && let Some(rest) = line.trim().strip_prefix(':')
        {
            let _ = rl.add_history_entry(&line);
            let mut parts = rest.splitn(2, char::is_whitespace);
            let command = parts.next().unwrap_or("");
            let rest = parts.next().unwrap_or("").trim();
            match command {
                "help" | "h" => print_help(),
                "doc" | "d" => run_doc_command(rest),
                "reset" => controller.reset(),
                "dump" => {
                    let dump = controller.dump();
                    if rest.is_empty() {
                        print!("{dump}");
                    } else if let Err(error) = std::fs::write(rest, dump) {
                        eprintln!("1q repl: {error}");
                    }
                }
                "to" => match DataFormat::from_str(rest) {
                    Ok(format) => match data::AnySerializer::from_format(
                        format,
                        Output::Stdout,
                        render_options.clone(),
                    ) {
                        Ok(next) => serializer = next,
                        Err(error) => eprintln!("1q repl: {error}"),
                    },
                    Err(_) => eprintln!("1q repl: unknown output format '{rest}'"),
                },
                "load" | "l" | "file" | "f" => {
                    let result = std::fs::read_to_string(rest)
                        .map_err(|e| e.to_string())
                        .and_then(|s| {
                            controller.load(
                                &s,
                                data::formats::extension::from_extension(rest)
                                    .unwrap_or(DataFormat::Json),
                            )
                        });
                    if let Err(error) = result {
                        eprintln!("1q repl: {error}");
                    }
                }
                "exit" | "quit" | "q" => break,
                _ => eprintln!("1q repl: unknown command :{command} (try :help)"),
            }
            continue;
        }
        if !buffer.is_empty() {
            buffer.push('\n');
        }
        buffer.push_str(&line);
        if parser::is_incomplete(&buffer) {
            continue;
        }
        let _ = rl.add_history_entry(&buffer);
        let filter = std::mem::take(&mut buffer);
        if filter.trim().is_empty() {
            continue;
        }
        match controller.evaluate(&filter) {
            Ok(values) => {
                for value in values {
                    if let Err(error) = serializer.put(value.clone()) {
                        eprintln!("1q repl: {error}");
                        break;
                    }
                }
            }
            Err(error) => eprintln!("1q repl: {error}"),
        }
    }
}
