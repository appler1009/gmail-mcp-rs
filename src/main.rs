mod auth;
mod config;
mod gmail;
mod http;
mod mcp;
mod tokens;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();

    let result = if args.get(1).map(String::as_str) == Some("--auth") {
        auth::run()
    } else {
        mcp::run()
    };

    if let Err(e) = result {
        eprintln!("gmail-mcp: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
