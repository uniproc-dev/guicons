use crate::position::LineIndex;
use crate::{display_path, Backend};
use guicons_core::{IconEntrySource, IconManifest};
use std::path::Path;
use tower_lsp::lsp_types::*;

/// Whether a manifest error should be published, given the
/// `reportTomlSyntaxErrors` setting - only raw TOML syntax errors are
/// ever suppressed, never semantic ones (unknown field, missing source, ...).
pub(crate) fn should_report_error(message: &str, report_toml_syntax_errors: bool) -> bool {
    report_toml_syntax_errors || !message.starts_with("TOML syntax error:")
}

/// `file`/`windows-ico` targets that don't exist on disk - not caught by
/// `guicons_core` at all (it only validates manifest *shape*), so this is
/// entirely an editor-tooling-side check.
pub(crate) fn missing_file_diagnostics(text: &str, path: &Path, manifest: &IconManifest, index: &LineIndex) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for entry in manifest.entries() {
        if entry.file() != path {
            continue;
        }
        if let IconEntrySource::File(target) = entry.source() {
            diagnostics.extend(missing_file_diagnostic(target, "file", entry.span(), text, index, manifest));
        }
        if let Some(ico) = entry.windows_ico() {
            diagnostics.extend(missing_file_diagnostic(ico, "windows-ico", entry.span(), text, index, manifest));
        }
    }
    diagnostics
}

fn missing_file_diagnostic(
    target: &Path,
    field: &str,
    span: std::ops::Range<usize>,
    text: &str,
    index: &LineIndex,
    manifest: &IconManifest,
) -> Option<Diagnostic> {
    if target.exists() {
        return None;
    }
    let mut message = format!("`{field}` not found: `{}`", display_path(target, manifest));
    if let Some(suggestion) = closest_file_name(target) {
        message.push_str(&format!(" - did you mean `{suggestion}`?"));
    }
    Some(Diagnostic {
        range: index.range(text, span),
        severity: Some(DiagnosticSeverity::ERROR),
        message,
        source: Some("guicons".to_string()),
        ..Default::default()
    })
}

/// `iconify = "provider:name"` entries whose SVG isn't cached locally yet -
/// same check `guicons-cli::check` does, mirrored here so the editor
/// doesn't fall behind the CLI. Informational, not a warning/error: not
/// being cached yet doesn't mean the id is wrong, just unconfirmed
/// without a network fetch (`icons fetch`), which neither `check` nor
/// this diagnostic ever does itself.
pub(crate) fn unresolved_iconify_diagnostics(text: &str, path: &Path, manifest: &IconManifest, index: &LineIndex) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for entry in manifest.entries() {
        if entry.file() != path {
            continue;
        }
        let IconEntrySource::Iconify(id) = entry.source() else { continue };

        let message = if id.split_once(':').is_none() {
            format!("iconify id `{id}` isn't in `provider:name` form - it will never resolve")
        } else {
            let cache_path = guicons_net::iconify_cache_path(manifest.workspace_root(), id);
            if cache_path.exists() {
                continue;
            }
            format!(
                "iconify icon `{id}` isn't cached locally yet - can't confirm it resolves without a network fetch (`icons fetch` or `GUICONS_ALLOW_NETWORK=1`)"
            )
        };

        diagnostics.push(Diagnostic {
            range: index.range(text, entry.span()),
            severity: Some(DiagnosticSeverity::INFORMATION),
            message,
            source: Some("guicons".to_string()),
            ..Default::default()
        });
    }
    diagnostics
}

/// Closest sibling file by name (Levenshtein distance), for a "did you
/// mean" suggestion - `None` if nothing in the directory is plausibly a
/// typo of `target`'s name (rather than just unrelated).
pub(crate) fn closest_file_name(target: &Path) -> Option<String> {
    let dir = target.parent()?;
    let target_name = target.file_name()?.to_string_lossy().to_string();
    let mut best: Option<(usize, String)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let distance = levenshtein(&target_name, &name);
        if best.as_ref().is_none_or(|(best_distance, _)| distance < *best_distance) {
            best = Some((distance, name));
        }
    }
    let (distance, name) = best?;
    (distance <= target_name.len().div_ceil(2).max(2)).then_some(name)
}

pub(crate) fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        curr[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

impl Backend {
    pub(crate) async fn publish_diagnostics_for(&self, uri: Url) {
        let Some(path) = Self::path_for_uri(&uri) else { return };
        let Some(text) = self.document_text(&uri).await else { return };

        let (manifest, errors) = guicons_core::load_icon_manifest_from_str(&path, &text);
        let index = LineIndex::new(&text);
        let report_syntax_errors = self.reports_toml_syntax_errors();
        let mut diagnostics: Vec<Diagnostic> = errors
            .iter()
            .filter(|error| error.file == path)
            .filter(|error| should_report_error(&error.message, report_syntax_errors))
            .map(|error| Diagnostic {
                range: match &error.span {
                    Some(span) => index.range(&text, span.clone()),
                    None => Range::new(Position::new(0, 0), Position::new(0, 0)),
                },
                severity: Some(DiagnosticSeverity::ERROR),
                message: error.message.clone(),
                source: Some("guicons".to_string()),
                ..Default::default()
            })
            .collect();
        diagnostics.extend(missing_file_diagnostics(&text, &path, &manifest, &index));
        diagnostics.extend(unresolved_iconify_diagnostics(&text, &path, &manifest, &index));

        self.client.publish_diagnostics(uri, diagnostics, None).await;
    }
}
