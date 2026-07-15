//! Lightweight text-based lookups the manifest model doesn't cover - table
//! header lines (which don't have their own span, unlike entries) and
//! `[include]` targets (resolved into `IconManifest` and then discarded).
//! Heuristic on purpose: good enough for hover/go-to-definition, not a
//! second parser.

/// If the line at `offset` is a `[family]`/`[family.24]` table header (not
/// `[include]`/`[defaults]`/`[providers.*]`), returns the family name and
/// optional size the same way the manifest parser would derive them from
/// that path.
pub fn family_header_at(text: &str, offset: usize) -> Option<(String, Option<u16>)> {
    let line = current_line(text, offset).trim();
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;
    if inner.is_empty() || inner == "include" || inner == "defaults" || inner.starts_with("providers") {
        return None;
    }

    let mut segments: Vec<&str> = inner.split('.').collect();
    let mut size = None;
    if let Some(last) = segments.last() {
        if let Ok(parsed) = last.parse::<u16>() {
            size = Some(parsed);
            segments.pop();
        }
    }
    if segments.is_empty() {
        return None;
    }
    Some((segments.join("-"), size))
}

/// If `offset` is on a `key = "value"` line inside the `[include]` table,
/// returns the (unresolved) path string.
pub fn include_target_at(text: &str, offset: usize) -> Option<String> {
    let (start, end) = include_section_range(text)?;
    if offset < start || offset >= end {
        return None;
    }
    let line = current_line(text, offset);
    let (_, value) = line.split_once('=')?;
    let value = value.trim().strip_prefix('"')?.strip_suffix('"')?;
    Some(value.to_string())
}

/// Byte range of the `[include]` table's body (after its own header line,
/// up to the next table header or end of file).
fn include_section_range(text: &str) -> Option<(usize, usize)> {
    let mut pos = 0usize;
    let mut body_start = None;
    for line in text.split_inclusive('\n') {
        let line_start = pos;
        pos += line.len();
        let trimmed = line.trim();
        if let Some(started_at) = body_start {
            if trimmed.starts_with('[') {
                return Some((started_at, line_start));
            }
            continue;
        }
        if trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) == Some("include") {
            body_start = Some(pos);
        }
    }
    body_start.map(|start| (start, text.len()))
}

fn current_line(text: &str, offset: usize) -> &str {
    let line_start = text[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = text[offset..].find('\n').map(|i| offset + i).unwrap_or(text.len());
    &text[line_start..line_end]
}
