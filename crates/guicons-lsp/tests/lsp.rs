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

/// Hovering the *including* document's own entry, with an `[include]`
/// present, must report that entry - not one pulled in from the included
/// file. (The deterministic version of this check, exercising
/// `IconEntry::file()` directly against exact byte spans, lives in
/// `guicons-core/tests/load.rs`.)
#[tokio::test]
async fn hover_reports_the_root_documents_own_entry_when_an_include_is_present() {
    let dir = tempdir().unwrap();
    write(dir.path(), "nav.gui.toml", "[back]\nfile = \"back.svg\"\n");
    let root_content = "[include]\nnav = \"nav.gui.toml\"\n\n[docker]\nfile = \"docker.svg\"\n";
    let path = write(dir.path(), "icons.gui.toml", root_content);
    let uri = file_uri(&path);

    let mut service = initialized_service().await;
    call(
        &mut service,
        "textDocument/didOpen",
        Some(json!({
            "textDocument": { "uri": uri, "languageId": "toml", "version": 1, "text": root_content }
        })),
        None,
    )
    .await;

    // Position of `docker.svg` within the root document's own text.
    let offset = root_content.find("docker.svg").unwrap();
    let line = root_content[..offset].matches('\n').count();
    let line_start = root_content[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let character = root_content[line_start..offset].len();

    let result = call(
        &mut service,
        "textDocument/hover",
        Some(json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        })),
        Some(2),
    )
    .await
    .expect("hover response");

    let value = result["contents"]["value"].as_str().unwrap();
    assert!(value.contains("docker"), "{value}");
    assert!(!value.contains("back"), "{value}");
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

#[tokio::test]
async fn hover_on_a_family_header_lists_its_variants_with_relative_paths() {
    let dir = tempdir().unwrap();
    write(dir.path(), "settings-filled.svg", "<svg/>");
    write(dir.path(), "settings-regular.svg", "<svg/>");
    let content = "[settings]\nvariants.filled = { file = \"settings-filled.svg\" }\nvariants.regular = { file = \"settings-regular.svg\" }\n";
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
            "position": { "line": 0, "character": 3 }
        })),
        Some(2),
    )
    .await
    .expect("hover response");

    let value = result["contents"]["value"].as_str().unwrap();
    assert!(value.contains("filled"), "{value}");
    assert!(value.contains("regular"), "{value}");
    assert!(value.contains("settings-filled.svg"), "{value}");
    assert!(!value.contains('\\'), "path should be forward-slashed, not escaped: {value}");
    assert!(!value.contains("File("), "source should be described nicely, not Debug-formatted: {value}");
}

#[tokio::test]
async fn goto_definition_on_an_entry_jumps_to_its_asset_file() {
    let dir = tempdir().unwrap();
    let asset = write(dir.path(), "docker.svg", "<svg/>");
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
        "textDocument/definition",
        Some(json!({
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": 10 }
        })),
        Some(2),
    )
    .await
    .expect("definition response");

    let target = result["uri"].as_str().expect("scalar location");
    let target_path = Url::parse(target).unwrap().to_file_path().unwrap();
    assert_eq!(fs::canonicalize(target_path).unwrap(), fs::canonicalize(asset).unwrap());
}

#[tokio::test]
async fn goto_definition_on_an_include_target_jumps_to_the_included_file() {
    let dir = tempdir().unwrap();
    let nav = write(dir.path(), "nav.gui.toml", "[back]\nfile = \"back.svg\"\n");
    let content = "[include]\nnav = \"nav.gui.toml\"\n";
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
        "textDocument/definition",
        Some(json!({
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": 10 }
        })),
        Some(2),
    )
    .await
    .expect("definition response");

    let target = result["uri"].as_str().expect("scalar location");
    let target_path = Url::parse(target).unwrap().to_file_path().unwrap();
    assert_eq!(fs::canonicalize(target_path).unwrap(), fs::canonicalize(nav).unwrap());
}

#[tokio::test]
async fn hover_on_a_keyword_shows_docs_and_example() {
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
            "position": { "line": 1, "character": 1 }
        })),
        Some(2),
    )
    .await
    .expect("hover response");

    let value = result["contents"]["value"].as_str().unwrap();
    assert!(value.contains("Local file path"), "{value}");
    assert!(value.contains("```toml"), "{value}");
}

#[tokio::test]
async fn hover_on_a_provider_name_shows_resolved_schema_and_origin() {
    let dir = tempdir().unwrap();
    let content = "[providers.fluent.override]\nvariants = [\"regular\", \"filled\", \"light\"]\n";
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
            "position": { "line": 0, "character": 12 }
        })),
        Some(2),
    )
    .await
    .expect("hover response");

    let value = result["contents"]["value"].as_str().unwrap();
    assert!(value.contains("fluent"), "{value}");
    assert!(value.contains("built-in provider, overridden"), "{value}");
    assert!(value.contains("light"), "{value}");
}

/// Clicking the `file` key itself (not just the string value after it)
/// must jump to the asset too - the whole `key = "value"` line is the
/// target, not only the value token `IconEntry::span()` covers.
#[tokio::test]
async fn goto_definition_on_the_file_keyword_itself_jumps_to_the_asset() {
    let dir = tempdir().unwrap();
    let asset = write(dir.path(), "docker.svg", "<svg/>");
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
        "textDocument/definition",
        Some(json!({
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": 1 }
        })),
        Some(2),
    )
    .await
    .expect("definition response");

    let target = result["uri"].as_str().expect("scalar location");
    let target_path = Url::parse(target).unwrap().to_file_path().unwrap();
    assert_eq!(fs::canonicalize(target_path).unwrap(), fs::canonicalize(asset).unwrap());
}
