use guicons_core::IconEntrySource;
use std::path::Path;

#[derive(Debug)]
pub struct FetchSummary {
    pub fetched: Vec<String>,
    pub skipped: Vec<String>,
    pub failed: Vec<(String, String)>,
}

impl FetchSummary {
    pub fn is_success(&self) -> bool {
        self.failed.is_empty()
    }
}

/// Populates `.cache/guicons/...` for every `iconify`/`url` entry in the
/// manifest. `cache_search_start` should be the current directory in
/// practice (see `main.rs`) - it's a parameter rather than read internally
/// so tests can point it at a tempdir instead of the real process cwd. It
/// must match whatever `guicons-build`'s codegen uses so a `fetch` here and
/// a later `cargo build` agree on where the icon lives.
pub fn fetch(manifest_path: &Path, cache_search_start: &Path, force: bool) -> Result<FetchSummary, Vec<String>> {
    let (manifest, errors) = guicons_core::load_icon_manifest(manifest_path);
    if !errors.is_empty() {
        return Err(errors.iter().map(|e| e.to_string()).collect());
    }

    let mut summary = FetchSummary {
        fetched: Vec::new(),
        skipped: Vec::new(),
        failed: Vec::new(),
    };

    for entry in manifest.entries() {
        let (cache_path, url, label) = match entry.source() {
            IconEntrySource::Iconify(id) => (
                guicons_net::iconify_cache_path(cache_search_start, id),
                guicons_net::iconify_url(id),
                id.clone(),
            ),
            IconEntrySource::Url(url) => {
                (guicons_net::url_cache_path(cache_search_start, url), url.clone(), url.clone())
            }
            IconEntrySource::File(_) | IconEntrySource::Glyph(_) => continue,
        };

        if cache_path.exists() && !force {
            summary.skipped.push(label);
            continue;
        }

        match guicons_net::download(&url, &cache_path) {
            Ok(()) => summary.fetched.push(label),
            Err(e) => summary.failed.push((label, e.to_string())),
        }
    }

    Ok(summary)
}
