use clap::Parser;
use oneq::{
    BuildConfig, cmd,
    cmd::flags::{self},
};
use std::io::IsTerminal;

fn main() {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    let args = if raw_args.iter().all(|arg| arg == "--") && std::io::stdin().is_terminal() {
        flags::Args::parse_from(["1q", "--repl"])
    } else {
        flags::Args::parse()
    };

    match args.cmd {
        Some(flags::Commands::BuildConfiguration) => {
            println!("{BuildConfig}");
            std::process::exit(0);
        }
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        Some(flags::Commands::Repl(..)) => cmd::repl::run(&args),
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        Some(flags::Commands::Repl(..)) => {
            eprintln!("1q: REPL is not supported in WASM");
            std::process::exit(2);
        }
        Some(flags::Commands::LSP) => {
            println!("1q lsp: UNIMPLEMENTED");
            std::process::exit(101);
        }
        None => cmd::jq::run(&args),
    }
}
