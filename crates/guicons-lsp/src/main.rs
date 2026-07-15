use tower_lsp::Server;

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = guicons_lsp::service();
    Server::new(stdin, stdout, socket).serve(service).await;
}
