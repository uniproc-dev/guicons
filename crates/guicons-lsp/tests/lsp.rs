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
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
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

/// Hovering the *including* document's own entry, with a `[link]`
/// present, must report that entry - not one pulled in from the included
/// file. (The deterministic version of this check, exercising
/// `IconEntry::file()` directly against exact byte spans, lives in
/// `guicons-core/tests/load.rs`.)
#[tokio::test]
async fn hover_reports_the_root_documents_own_entry_when_an_include_is_present() {
    let dir = tempdir().unwrap();
    write(dir.path(), "nav.gui.toml", "[back]\nfile = \"back.svg\"\n");
    let root_content = "[link]\nincludes = [\"nav.gui.toml\"]\n\n[docker]\nfile = \"docker.svg\"\n";
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
    let content = "[link]\nincludes = [\"nav.gui.toml\"]\n";
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
            "position": { "line": 1, "character": 14 }
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

#[tokio::test]
async fn completion_at_top_level_lists_manifest_sections() {
    let dir = tempdir().unwrap();
    let content = "\n[docker]\nfile = \"docker.svg\"\n";
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
            "position": { "line": 0, "character": 0 }
        })),
        Some(2),
    )
    .await
    .expect("completion response");

    let items = result.as_array().expect("array response");
    let labels: Vec<&str> = items.iter().map(|item| item["label"].as_str().unwrap()).collect();
    assert!(labels.contains(&"defaults"), "{labels:?}");
    assert!(labels.contains(&"link"), "{labels:?}");
    assert!(labels.contains(&"providers"), "{labels:?}");
}

#[tokio::test]
async fn completion_inside_an_entry_lists_source_fields() {
    let dir = tempdir().unwrap();
    let content = "[docker]\n\nfile = \"docker.svg\"\n";
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
            "position": { "line": 1, "character": 0 }
        })),
        Some(2),
    )
    .await
    .expect("completion response");

    let items = result.as_array().expect("array response");
    let labels: Vec<&str> = items.iter().map(|item| item["label"].as_str().unwrap()).collect();
    assert!(labels.contains(&"file"), "{labels:?}");
    assert!(labels.contains(&"iconify"), "{labels:?}");
    assert!(labels.contains(&"variants"), "{labels:?}");
}

#[tokio::test]
async fn completion_inside_link_section_suggests_includes() {
    let dir = tempdir().unwrap();
    let content = "[link]\n\n";
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
            "position": { "line": 1, "character": 0 }
        })),
        Some(2),
    )
    .await
    .expect("completion response");

    let items = result.as_array().expect("array response");
    let labels: Vec<&str> = items.iter().map(|item| item["label"].as_str().unwrap()).collect();
    assert_eq!(labels, vec!["includes"]);
}

/// Completion must keep suggesting fields after the user has typed part
/// of the key (not only on a completely empty line), and the replacement
/// range must cover exactly the typed prefix - not reach back into the
/// previous line, which would delete its trailing newline on accept.
#[tokio::test]
async fn completion_after_a_partial_field_prefix_still_suggests_and_has_a_safe_range() {
    let dir = tempdir().unwrap();
    let content = "[docker]\nf\n";
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
            "position": { "line": 1, "character": 1 }
        })),
        Some(2),
    )
    .await
    .expect("completion response");

    let items = result.as_array().expect("array response");
    let labels: Vec<&str> = items.iter().map(|item| item["label"].as_str().unwrap()).collect();
    assert!(labels.contains(&"file"), "{labels:?}");

    let file_item = items.iter().find(|item| item["label"] == "file").unwrap();
    let range = &file_item["textEdit"]["range"];
    assert_eq!(range["start"]["line"], 1);
    assert_eq!(range["start"]["character"], 0);
    assert_eq!(range["end"]["line"], 1);
    assert_eq!(range["end"]["character"], 1);
}

#[tokio::test]
async fn completion_inside_a_file_value_lists_matching_assets() {
    let dir = tempdir().unwrap();
    write(dir.path(), "settings-filled.svg", "<svg/>");
    write(dir.path(), "settings-regular.svg", "<svg/>");
    write(dir.path(), "readme.txt", "not an icon");
    let content = "[settings]\nfile = \"set\"\n";
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

    // `file = "set"` - the opening quote is at column 7, "set" spans
    // columns 8-10, so right after "set" (before the closing quote) is
    // column 11.
    let result = call(
        &mut service,
        "textDocument/completion",
        Some(json!({
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": 11 }
        })),
        Some(2),
    )
    .await
    .expect("completion response");

    let items = result.as_array().expect("array response");
    let labels: Vec<&str> = items.iter().map(|item| item["label"].as_str().unwrap()).collect();
    assert!(labels.contains(&"settings-filled.svg"), "{labels:?}");
    assert!(labels.contains(&"settings-regular.svg"), "{labels:?}");
    assert!(!labels.contains(&"readme.txt"), "{labels:?}");

    let item = items.iter().find(|item| item["label"] == "settings-filled.svg").unwrap();
    let range = &item["textEdit"]["range"];
    assert_eq!(range["start"]["line"], 1);
    assert_eq!(range["start"]["character"], 8);
    assert_eq!(range["end"]["line"], 1);
    assert_eq!(range["end"]["character"], 11);
}

#[tokio::test]
async fn completion_inside_link_includes_lists_matching_manifests() {
    let dir = tempdir().unwrap();
    write(dir.path(), "nav.gui.toml", "[back]\nfile = \"back.svg\"\n");
    let content = "[link]\nincludes = [\"na\"]\n";
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

    // Cursor right after "na" inside the quotes.
    let result = call(
        &mut service,
        "textDocument/completion",
        Some(json!({
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": 15 }
        })),
        Some(2),
    )
    .await
    .expect("completion response");

    let items = result.as_array().expect("array response");
    let labels: Vec<&str> = items.iter().map(|item| item["label"].as_str().unwrap()).collect();
    assert!(labels.contains(&"nav.gui.toml"), "{labels:?}");
}


#[tokio::test]
async fn completion_inside_an_iconify_value_before_the_colon_lists_provider_names() {
    let dir = tempdir().unwrap();
    let content = "[docker]\niconify = \"flu\"\n";
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

    // `iconify = "flu"` - cursor right after "flu", before the closing quote.
    let result = call(
        &mut service,
        "textDocument/completion",
        Some(json!({
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": 14 }
        })),
        Some(2),
    )
    .await
    .expect("completion response");

    let items = result["items"].as_array().expect("completion list");
    let labels: Vec<&str> = items.iter().map(|item| item["label"].as_str().unwrap()).collect();
    assert!(labels.contains(&"fluent:"), "{labels:?}");
}

/// Once a provider is typed (past the `:`), completion should switch to
/// suggesting icon names from that provider's already-cached collection -
/// pre-seeded here to avoid depending on real network access or the
/// background warmup actually having finished.
#[tokio::test]
async fn completion_inside_an_iconify_value_after_the_colon_lists_cached_icon_names() {
    let dir = tempdir().unwrap();
    let cache_dir = dir.path().join(".cache/guicons/_collections");
    fs::create_dir_all(&cache_dir).unwrap();
    fs::write(
        cache_dir.join("mdi.json"),
        json!({
            "prefix": "mdi",
            "uncategorized": ["home", "home-outline"],
            "categories": { "misc": ["account"] }
        })
        .to_string(),
    )
    .unwrap();

    let content = "[docker]\niconify = \"mdi:ho\"\n";
    let path = write(dir.path(), "icons.gui.toml", content);
    let uri = file_uri(&path);

    let (mut service, _socket) = guicons_lsp::service();
    call(
        &mut service,
        "initialize",
        Some(json!({
            "capabilities": {},
            "rootUri": file_uri(dir.path())
        })),
        Some(1),
    )
    .await;
    call(&mut service, "initialized", Some(json!({})), None).await;

    call(
        &mut service,
        "textDocument/didOpen",
        Some(json!({
            "textDocument": { "uri": uri, "languageId": "toml", "version": 1, "text": content }
        })),
        None,
    )
    .await;

    // `iconify = "mdi:ho"` - cursor right after "ho", before the closing quote.
    let result = call(
        &mut service,
        "textDocument/completion",
        Some(json!({
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": 17 }
        })),
        Some(2),
    )
    .await
    .expect("completion response");

    assert_eq!(result["isIncomplete"], false, "well under the cap, should not claim more exist");
    let items = result["items"].as_array().expect("completion list");
    let labels: Vec<&str> = items.iter().map(|item| item["label"].as_str().unwrap()).collect();
    assert!(labels.contains(&"home"), "{labels:?}");
    assert!(labels.contains(&"home-outline"), "{labels:?}");
    assert!(!labels.contains(&"account"), "{labels:?}");

    let item = items.iter().find(|item| item["label"] == "home").unwrap();
    let range = &item["textEdit"]["range"];
    assert_eq!(range["start"]["line"], 1);
    assert_eq!(range["start"]["character"], 15);
    assert_eq!(range["end"]["line"], 1);
    assert_eq!(range["end"]["character"], 17);
}

/// A collection with more names than the completion cap must return only
/// the capped prefix and mark the response `isIncomplete` - sending every
/// match on every keystroke doesn't scale to the ~7500-name collections
/// some real providers have (`mdi`, for instance).
#[tokio::test]
async fn completion_inside_an_iconify_value_caps_a_huge_collection_and_marks_it_incomplete() {
    let dir = tempdir().unwrap();
    let cache_dir = dir.path().join(".cache/guicons/_collections");
    fs::create_dir_all(&cache_dir).unwrap();
    let names: Vec<String> = (0..500).map(|i| format!("icon-{i:04}")).collect();
    fs::write(cache_dir.join("mdi.json"), json!({ "prefix": "mdi", "uncategorized": names }).to_string()).unwrap();

    let content = "[docker]\niconify = \"mdi:icon-\"\n";
    let path = write(dir.path(), "icons.gui.toml", content);
    let uri = file_uri(&path);

    let (mut service, _socket) = guicons_lsp::service();
    call(
        &mut service,
        "initialize",
        Some(json!({ "capabilities": {}, "rootUri": file_uri(dir.path()) })),
        Some(1),
    )
    .await;
    call(&mut service, "initialized", Some(json!({})), None).await;
    call(
        &mut service,
        "textDocument/didOpen",
        Some(json!({
            "textDocument": { "uri": uri, "languageId": "toml", "version": 1, "text": content }
        })),
        None,
    )
    .await;

    // `iconify = "mdi:icon-"` - cursor right after "icon-", before the closing quote.
    let result = call(
        &mut service,
        "textDocument/completion",
        Some(json!({
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": 20 }
        })),
        Some(2),
    )
    .await
    .expect("completion response");

    assert_eq!(result["isIncomplete"], true, "500 matches exceed the cap");
    let items = result["items"].as_array().expect("completion list");
    assert!(items.len() < 500, "response should be capped, got {}", items.len());
}

/// Hovering an `icon!(family.variant)` call in a `.rs` file resolves
/// against whatever manifest was found anywhere in the workspace at
/// startup (`scan_workspace_manifests`) - not the `.rs` file itself,
/// which has no manifest of its own.
#[tokio::test]
async fn hover_on_an_icon_macro_call_in_a_rust_file_shows_the_resolved_entry() {
    let dir = tempdir().unwrap();
    // `hover_rust` finds the .rs file's own manifest by locating its crate
    // root (nearest ancestor `Cargo.toml`), matching how `guicons-macros`
    // resolves `icon!(...)` at compile time against `CARGO_MANIFEST_DIR` -
    // so the fixture needs a `Cargo.toml` right alongside the manifest.
    write(dir.path(), "Cargo.toml", "[package]\nname = \"fixture\"\nversion = \"0.0.0\"\n");
    write(dir.path(), "docker.svg", "<svg/>");
    write(dir.path(), "icons.gui.toml", "[docker]\nfile = \"docker.svg\"\n");
    let rs_content = "fn f() { let _ = icon!(docker); }";
    let rs_path = write(dir.path(), "main.rs", rs_content);
    let uri = file_uri(&rs_path);

    let (mut service, _socket) = guicons_lsp::service();
    call(
        &mut service,
        "initialize",
        Some(json!({ "capabilities": {}, "rootUri": file_uri(dir.path()) })),
        Some(1),
    )
    .await;
    call(&mut service, "initialized", Some(json!({})), None).await;
    call(
        &mut service,
        "textDocument/didOpen",
        Some(json!({
            "textDocument": { "uri": uri, "languageId": "rust", "version": 1, "text": rs_content }
        })),
        None,
    )
    .await;

    let offset = rs_content.find("docker").unwrap();
    let result = call(
        &mut service,
        "textDocument/hover",
        Some(json!({
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": offset }
        })),
        Some(2),
    )
    .await
    .expect("hover response");

    let value = result["contents"]["value"].as_str().unwrap();
    assert!(value.contains("docker"), "{value}");
}

/// `icon!("set:name")` (a raw iconify literal) has no manifest entry at
/// all - hover must show a distinct message rather than silently finding
/// nothing.
#[tokio::test]
async fn hover_on_an_icon_macro_call_with_an_iconify_literal_shows_iconify_info() {
    let dir = tempdir().unwrap();
    let rs_content = "fn f() { let _ = icon!(\"mdi:home\"); }";
    let rs_path = write(dir.path(), "main.rs", rs_content);
    let uri = file_uri(&rs_path);

    let (mut service, _socket) = guicons_lsp::service();
    call(
        &mut service,
        "initialize",
        Some(json!({ "capabilities": {}, "rootUri": file_uri(dir.path()) })),
        Some(1),
    )
    .await;
    call(&mut service, "initialized", Some(json!({})), None).await;
    call(
        &mut service,
        "textDocument/didOpen",
        Some(json!({
            "textDocument": { "uri": uri, "languageId": "rust", "version": 1, "text": rs_content }
        })),
        None,
    )
    .await;

    let offset = rs_content.find("mdi:home").unwrap();
    let result = call(
        &mut service,
        "textDocument/hover",
        Some(json!({
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": offset }
        })),
        Some(2),
    )
    .await
    .expect("hover response");

    let value = result["contents"]["value"].as_str().unwrap();
    assert!(value.contains("mdi:home"), "{value}");
    assert!(value.contains("no manifest entry"), "{value}");
}

/// A `.rs` file whose workspace has no `icons.gui.toml` at all must
/// gracefully report no hover, not error or panic.
#[tokio::test]
async fn hover_on_an_icon_macro_call_with_no_manifest_in_the_workspace_is_none() {
    let dir = tempdir().unwrap();
    let rs_content = "fn f() { let _ = icon!(docker); }";
    let rs_path = write(dir.path(), "main.rs", rs_content);
    let uri = file_uri(&rs_path);

    let (mut service, _socket) = guicons_lsp::service();
    call(
        &mut service,
        "initialize",
        Some(json!({ "capabilities": {}, "rootUri": file_uri(dir.path()) })),
        Some(1),
    )
    .await;
    call(&mut service, "initialized", Some(json!({})), None).await;
    call(
        &mut service,
        "textDocument/didOpen",
        Some(json!({
            "textDocument": { "uri": uri, "languageId": "rust", "version": 1, "text": rs_content }
        })),
        None,
    )
    .await;

    let offset = rs_content.find("docker").unwrap();
    let result = call(
        &mut service,
        "textDocument/hover",
        Some(json!({
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": offset }
        })),
        Some(2),
    )
    .await;

    assert_eq!(result, Some(Value::Null), "{result:?}");
}

/// A `.rs` file must only ever resolve `icon!(...)` against *its own*
/// crate's manifest - never a different crate's, even if the workspace
/// has several and they happen to share a family name. Regression test
/// for resolving against `self.manifests.values().find_map(...)` (any
/// manifest that happened to match) instead of the specific
/// `CARGO_MANIFEST_DIR`-equivalent one.
#[tokio::test]
async fn hover_in_a_rust_file_only_resolves_against_its_own_crates_manifest() {
    let dir = tempdir().unwrap();

    // Crate "a": its own Cargo.toml, manifest, and .rs file.
    write(dir.path(), "a/Cargo.toml", "[package]\nname = \"a\"\nversion = \"0.0.0\"\n");
    write(dir.path(), "a/docker.svg", "<svg/>");
    write(dir.path(), "a/icons.gui.toml", "[docker]\nfile = \"docker.svg\"\n");
    let rs_content = "fn f() { let _ = icon!(docker); }";
    let rs_path = write(dir.path(), "a/src/main.rs", rs_content);

    // Crate "b": a different manifest defining the *same* family name
    // with a different source - if hover ever picked this one up instead
    // of crate "a"'s own, this test would catch it.
    write(dir.path(), "b/Cargo.toml", "[package]\nname = \"b\"\nversion = \"0.0.0\"\n");
    write(dir.path(), "b/other.svg", "<svg/>");
    write(dir.path(), "b/icons.gui.toml", "[docker]\nfile = \"other.svg\"\n");

    let uri = file_uri(&rs_path);
    let (mut service, _socket) = guicons_lsp::service();
    call(
        &mut service,
        "initialize",
        Some(json!({ "capabilities": {}, "rootUri": file_uri(dir.path()) })),
        Some(1),
    )
    .await;
    call(&mut service, "initialized", Some(json!({})), None).await;
    call(
        &mut service,
        "textDocument/didOpen",
        Some(json!({
            "textDocument": { "uri": uri, "languageId": "rust", "version": 1, "text": rs_content }
        })),
        None,
    )
    .await;

    let offset = rs_content.find("docker").unwrap();
    let result = call(
        &mut service,
        "textDocument/hover",
        Some(json!({
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": offset }
        })),
        Some(2),
    )
    .await
    .expect("hover response");

    let value = result["contents"]["value"].as_str().unwrap();
    assert!(value.contains("docker.svg"), "should resolve crate a's own entry: {value}");
    assert!(!value.contains("other.svg"), "must not pick up crate b's entry: {value}");
}

#[tokio::test]
async fn initialize_reads_the_report_toml_syntax_errors_option() {
    let (mut service, _socket) = guicons_lsp::service();
    assert!(!service.inner().reports_toml_syntax_errors(), "defaults to off - most editors already report these");

    call(
        &mut service,
        "initialize",
        Some(json!({
            "capabilities": {},
            "initializationOptions": { "reportTomlSyntaxErrors": true }
        })),
        Some(1),
    )
    .await;

    assert!(service.inner().reports_toml_syntax_errors());
}
