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

/// Whether `offset`'s line overlaps `span` - used to make an entry's whole
/// `key = "value"` line a go-to-definition target, not just the value
/// token `IconEntry::span()` itself covers (so clicking on `file`, not
/// just the string after it, still jumps to the source file).
pub fn offset_line_overlaps(text: &str, offset: usize, span: std::ops::Range<usize>) -> bool {
    let line_start = text[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = text[offset..].find('\n').map(|i| offset + i).unwrap_or(text.len());
    span.start < line_end && span.end > line_start
}

/// One-liner + example for a manifest keyword, shown on hover.
pub struct KeywordDoc {
    pub description: &'static str,
    pub example: &'static str,
}

const KEYWORD_DOCS: &[(&str, KeywordDoc)] = &[
    ("defaults", KeywordDoc {
        description: "Shared defaults inherited by every entry in this file: `root`, `provider`, `size`.",
        example: "[defaults]\nroot = \"assets/icons\"\nprovider = \"fluent\"\nsize = 24",
    }),
    ("root", KeywordDoc {
        description: "Base directory `file`/`windows-ico` paths resolve against.",
        example: "root = \"assets/icons\"",
    }),
    ("provider", KeywordDoc {
        description: "Default iconify provider prefix used to synthesize an `iconify` id when an entry gives none explicitly.",
        example: "provider = \"fluent\"",
    }),
    ("size", KeywordDoc {
        description: "Default size: feeds the synthesized `iconify` id, and is inherited by entries without their own `[family.N]` numeric segment.",
        example: "size = 24",
    }),
    ("include", KeywordDoc {
        description: "Other manifest files to merge in: `name = \"path/to/file.gui.toml\"`, resolved relative to this file.",
        example: "[include]\nnav = \"icons/nav.gui.toml\"",
    }),
    ("providers", KeywordDoc {
        description: "Provider schemas (`variants`/`sizes`) used to reverse-parse a pasted iconify id into family/size/variant.",
        example: "[providers.fluent]\nvariants = [\"regular\", \"filled\"]\nsizes = [16, 20, 24]",
    }),
    ("variants", KeywordDoc {
        description: "Named variants for this family, each an inline table with its own source.",
        example: "variants.filled = { file = \"settings-filled.svg\" }",
    }),
    ("sizes", KeywordDoc {
        description: "Sizes this provider's icons come in - lets `decompose_iconify_id` tell a size suffix apart from the icon's own name.",
        example: "sizes = [16, 20, 24, 28, 32, 48]",
    }),
    ("override", KeywordDoc {
        description: "Per-field override of a built-in provider's schema - fields left out here inherit the builtin's value.",
        example: "[providers.fluent.override]\nvariants = [\"regular\", \"filled\", \"light\"]",
    }),
    ("file", KeywordDoc {
        description: "Local file path, resolved against `defaults.root`.",
        example: "file = \"settings.svg\"",
    }),
    ("iconify", KeywordDoc {
        description: "An iconify.design id (`provider:name`) - fetched and cached offline the first time it's needed.",
        example: "iconify = \"fluent:settings-24-regular\"",
    }),
    ("url", KeywordDoc {
        description: "A remote URL, fetched and cached offline (always treated as SVG).",
        example: "url = \"https://example.com/icon.svg\"",
    }),
    ("glyph", KeywordDoc {
        description: "A font glyph spec `font-family:codepoint` (`codepoint` is a literal character or a `U+XXXX` hex escape).",
        example: "glyph = \"MyIconFont:U+E001\"",
    }),
    ("windows-ico", KeywordDoc {
        description: "Alternate source file used specifically when generating a Windows `.ico`.",
        example: "windows-ico = \"settings.ico\"",
    }),
    ("dynamic", KeywordDoc {
        description: "Marks this entry as only resolvable at runtime, not through compile-time codegen assumptions.",
        example: "dynamic = true",
    }),
];

/// If `offset` sits on a manifest keyword used as a key or a table-header
/// segment (not inside a string value), returns the keyword and its doc.
pub fn keyword_at(text: &str, offset: usize) -> Option<(&'static str, &'static KeywordDoc)> {
    let line = current_line(text, offset);
    let col = offset - line_start_of(text, offset);
    if is_inside_string_literal(line, col) {
        return None;
    }
    let (word, _, _) = word_at(line, col)?;
    KEYWORD_DOCS.iter().find(|(name, _)| *name == word).map(|(name, doc)| (*name, doc))
}

/// If the line at `offset` is a `[providers.<name>]`/`[providers.<name>.override]`
/// header, returns `<name>`.
pub fn provider_name_at(text: &str, offset: usize) -> Option<String> {
    let line = current_line(text, offset).trim();
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;
    let rest = inner.strip_prefix("providers.")?;
    let name = rest.strip_suffix(".override").unwrap_or(rest);
    if name.is_empty() {
        return None;
    }
    Some(name.to_string())
}

fn line_start_of(text: &str, offset: usize) -> usize {
    text[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0)
}

/// Extracts the identifier-like word touching column `col` in `line`
/// (`-`/`_` included, so `windows-ico` is one word), plus its start/end
/// columns.
fn word_at(line: &str, col: usize) -> Option<(&str, usize, usize)> {
    let is_word_char = |c: char| c.is_alphanumeric() || c == '-' || c == '_';
    let bytes_len = line.len();
    let col = col.min(bytes_len);

    let start = line[..col].rfind(|c| !is_word_char(c)).map(|i| i + 1).unwrap_or(0);
    let end = line[col..].find(|c| !is_word_char(c)).map(|i| col + i).unwrap_or(bytes_len);
    if start >= end {
        return None;
    }
    Some((&line[start..end], start, end))
}

fn is_inside_string_literal(line: &str, col: usize) -> bool {
    line[..col.min(line.len())].matches('"').count() % 2 == 1
}
