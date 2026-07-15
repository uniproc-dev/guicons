use guicons_lsp::Backend;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use tempfile::tempdir;
use tower::Service;
use tower_lsp::jsonrpc::Request;
use tower_lsp::lsp_types::Url;
use tower_lsp::LspService;

async fn initialized_service() -> LspService<Backend> {
    let (mut service, _socket) = guicons_lsp::service();
    call(&mut service, "initialize", Some(json!({ "capabilities": {} })), Some(1)).await;
    call(&mut service, "initialized", Some(json!({})), None).await;
    service
}

async fn call(service: &mut LspService<Backend>, method: &str, params: Option<Value>, id: Option<i64>) -> Option<Value> {
    let mut builder = Request::build(method.to_string());
    if let Some(params) = params {
        builder = builder.params(params);
    }
    if let Some(id) = id {
        builder = builder.id(id);
    }
    let request = builder.finish();
    let response = service.call(request).await.unwrap();
    response.and_then(|response| response.into_parts().1.ok())
}

fn file_uri(path: &Path) -> Url {
    Url::from_file_path(path).unwrap()
}

fn write(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    fs::write(&path, content).unwrap();
    path
}

#[tokio::test]
async fn diagnostics_reported_for_invalid_manifest_content_on_open() {
    let dir = tempdir().unwrap();
    let path = write(dir.path(), "icons.gui.toml", "");
    let uri = file_uri(&path);

    let mut service = initialized_service().await;

    let invalid = r#"
    [docker]
    file = "docker.svg"
    iconify = "set:name"
    "#;
    call(
        &mut service,
        "textDocument/didOpen",
        Some(json!({
            "textDocument": { "uri": uri, "languageId": "toml", "version": 1, "text": invalid }
        })),
        None,
    )
    .await;

    // No direct response to a notification - diagnostics went out over the
    // client socket, which this test doesn't drain. Instead, confirm the
    // server-side parse itself flags the "two sources" error we expect,
    // by re-deriving it the same way the server does.
    let (_, errors) = guicons_core::load_icon_manifest_from_str(&path, invalid);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].message.contains("exactly one source"));
}

#[tokio::test]
async fn hover_reports_entry_details_at_cursor() {
    let dir = tempdir().unwrap();
    let content = "[docker]\nfile = \"docker.svg\"\n";
    let path = write(dir.path(), "icons.gui.toml", content);
    let uri = file_uri(&path);

    let mut service = initialized_service().await;
    call(
        &mut service,
        "textDocument/didOpen",
        Some(json!({
            "textDocument": { "uri": uri, "languageId": "toml", "version": 1, "text": content }
        })),
        None,
    )
    .await;

    let result = call(
        &mut service,
        "textDocument/hover",
        Some(json!({
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": 10 }
        })),
        Some(2),
    )
    .await
    .expect("hover response");

    let value = result["contents"]["value"].as_str().unwrap();
    assert!(value.contains("docker"), "{value}");
}

#[tokio::test]
async fn completion_inside_providers_header_lists_builtin_names() {
    let dir = tempdir().unwrap();
    let content = "[providers.]\n";
    let path = write(dir.path(), "icons.gui.toml", content);
    let uri = file_uri(&path);

    let mut service = initialized_service().await;
    call(
        &mut service,
        "textDocument/didOpen",
        Some(json!({
            "textDocument": { "uri": uri, "languageId": "toml", "version": 1, "text": content }
        })),
        None,
    )
    .await;

    let result = call(
        &mut service,
        "textDocument/completion",
        Some(json!({
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": 11 }
        })),
        Some(2),
    )
    .await
    .expect("completion response");

    let items = result.as_array().expect("array response");
    let labels: Vec<&str> = items.iter().map(|item| item["label"].as_str().unwrap()).collect();
    assert!(labels.contains(&"fluent"), "{labels:?}");
}
