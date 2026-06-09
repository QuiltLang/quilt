//! `quilt-lsp` binary: serve the Language Server over stdio.

use quilt_lsp::server::Backend;
use tower_lsp::{LspService, Server};

#[tokio::main]
async fn main() {
    // The LSP protocol owns stdout, so all logs must go to stderr.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
