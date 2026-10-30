//! `bedrock-mcp --vault <folder>`: a local MCP server over stdio, for Claude
//! Desktop, Claude Code and other MCP clients.
//!
//!     claude mcp add bedrock -- bedrock-mcp --vault "C:\Notes"

use std::path::PathBuf;

use bedrock_core::Vault;
use bedrock_mcp::local::LocalBackend;
use bedrock_mcp::protocol;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn vault_path() -> Option<PathBuf> {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--vault" {
            return args.next().map(PathBuf::from);
        }
        if let Some(v) = a.strip_prefix("--vault=") {
            return Some(PathBuf::from(v));
        }
    }
    std::env::var_os("BEDROCK_VAULT").map(PathBuf::from)
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let Some(path) = vault_path() else {
        eprintln!("usage: bedrock-mcp --vault <folder>   (or set BEDROCK_VAULT)");
        std::process::exit(2);
    };
    let vault = match Vault::open(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("bedrock-mcp: cannot open vault {}: {e}", path.display());
            std::process::exit(1);
        }
    };
    let backend = LocalBackend::new(vault);

    // Messages are newline-delimited JSON; stdout carries only responses.
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str(&line) {
            Ok(msg) => protocol::handle(&backend, msg).await,
            Err(e) => Some(protocol::parse_error(&e.to_string())),
        };
        if let Some(reply) = reply {
            let mut out = reply.to_string();
            out.push('\n');
            if stdout.write_all(out.as_bytes()).await.is_err() || stdout.flush().await.is_err() {
                break;
            }
        }
    }
}
