use std::fs;
use std::path::Path;
use tempfile::tempdir;

fn write(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, content).unwrap();
    path
}

/// Starts a mock HTTP server on an ephemeral local port that serves `body`
/// to exactly one request, then shuts down. Returns the base URL and a
/// join handle the caller should wait on after triggering the request.
fn spawn_mock_server(body: &'static str) -> (String, std::thread::JoinHandle<()>) {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", server.server_addr());
    let handle = std::thread::spawn(move || {
        if let Ok(request) = server.recv() {
            let _ = request.respond(tiny_http::Response::from_string(body));
        }
    });
    (base_url, handle)
}

#[test]
fn already_cached_icons_are_skipped_without_force() {
    let dir = tempdir().unwrap();
    // A workspace root marker so guicons-net's cache dir lands inside `dir`.
    write(dir.path(), "Cargo.toml", "[workspace]\n");
    let manifest = write(
        dir.path(),
        "icons.gui.toml",
        r#"
        [settings]
        iconify = "fluent:settings-24-regular"
        "#,
    );
    write(
        dir.path(),
        ".cache/guicons/fluent/settings-24-regular.svg",
        "<svg></svg>",
    );

    let summary = guicons_cli::fetch(&manifest, dir.path(), false).unwrap();
    assert_eq!(summary.skipped, vec!["fluent:settings-24-regular"]);
    assert!(summary.fetched.is_empty());
    assert!(summary.failed.is_empty());
    assert!(summary.is_success());
}

#[test]
fn unreachable_url_is_reported_as_failed_not_a_panic() {
    let dir = tempdir().unwrap();
    write(dir.path(), "Cargo.toml", "[workspace]\n");
    // `.invalid` is a reserved TLD (RFC 2606) guaranteed to never resolve,
    // so this fails deterministically regardless of the test machine's
    // actual network access.
    let manifest = write(
        dir.path(),
        "icons.gui.toml",
        r#"
        [settings]
        url = "http://guicons-test.invalid/settings.svg"

        [docker]
        url = "http://guicons-test.invalid/docker.svg"
        "#,
    );

    // Both icons should fail to download, and both should be reported -
    // not just the first one, and no panic.
    let summary = guicons_cli::fetch(&manifest, dir.path(), false).unwrap();
    assert!(summary.fetched.is_empty());
    assert!(summary.skipped.is_empty());
    assert_eq!(summary.failed.len(), 2);
    assert!(!summary.is_success());
}

#[test]
fn fetch_downloads_a_url_source_from_a_real_http_response() {
    let dir = tempdir().unwrap();
    write(dir.path(), "Cargo.toml", "[workspace]\n");

    let svg = "<svg><circle/></svg>";
    let (base_url, handle) = spawn_mock_server(svg);
    let url = format!("{base_url}/settings.svg");
    let manifest = write(dir.path(), "icons.gui.toml", &format!("[settings]\nurl = \"{url}\"\n"));

    let summary = guicons_cli::fetch(&manifest, dir.path(), false).unwrap();
    handle.join().unwrap();

    assert_eq!(summary.fetched, vec![url.clone()]);
    assert!(summary.skipped.is_empty());
    assert!(summary.failed.is_empty(), "{:?}", summary.failed);

    let cache_path = guicons_net::url_cache_path(dir.path(), &url);
    assert_eq!(fs::read_to_string(&cache_path).unwrap(), svg);
}

#[test]
fn fetch_redownloads_a_cached_url_when_forced() {
    let dir = tempdir().unwrap();
    write(dir.path(), "Cargo.toml", "[workspace]\n");

    let svg = "<svg><rect/></svg>";
    let (base_url, handle) = spawn_mock_server(svg);
    let url = format!("{base_url}/settings.svg");
    let manifest = write(dir.path(), "icons.gui.toml", &format!("[settings]\nurl = \"{url}\"\n"));

    let cache_path = guicons_net::url_cache_path(dir.path(), &url);
    fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
    fs::write(&cache_path, "<svg>stale</svg>").unwrap();

    let summary = guicons_cli::fetch(&manifest, dir.path(), true).unwrap();
    handle.join().unwrap();

    assert_eq!(summary.fetched, vec![url]);
    assert!(summary.skipped.is_empty());
    assert!(summary.failed.is_empty(), "{:?}", summary.failed);
    assert_eq!(fs::read_to_string(&cache_path).unwrap(), svg);
}

#[test]
fn manifest_parse_errors_are_returned_not_panicked_on() {
    let dir = tempdir().unwrap();
    let manifest = write(
        dir.path(),
        "icons.gui.toml",
        r#"
        [settings]
        bogus = "oops"
        "#,
    );

    let errors = guicons_cli::fetch(&manifest, dir.path(), false).unwrap_err();
    assert!(!errors.is_empty());
}
