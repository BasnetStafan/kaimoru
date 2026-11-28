//! Binary entry point for the stdio MCP adapter: reads
//! the environment, sends logs to stderr (stdout is the
//! protocol channel and carries JSON-RPC only) and serves
//! until the client disconnects.

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .init();
    match mcp_stdio::run(mcp_stdio::Config::from_env()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("mcp-stdio: {error}");
            ExitCode::FAILURE
        }
    }
}
