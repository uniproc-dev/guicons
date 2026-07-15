//! Lightweight text-based lookups the manifest model doesn't cover - table
//! header lines (which don't have their own span, unlike entries) and
//! `[link] includes = [...]` targets (resolved into `IconManifest` and then
//! discarded). Heuristic on purpose: good enough for hover/go-to-definition,
//! not a second parser.

/// If the line at `offset` is a `[family]`/`[family.24]` table header (not
/// `[link]`/`[defaults]`/`[providers.*]`), returns the family name and
/// optional size the same way the manifest parser would derive them from
/// that path.
pub fn family_header_at(text: &str, offset: usize) -> Option<(String, Option<u16>)> {
    let line = current_line(text, offset).trim();
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;
    if inner.is_empty() || inner == "link" || inner == "defaults" || inner.starts_with("providers") {
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

/// Which kind of table `offset` falls under, found by scanning backwards
/// for the nearest preceding table header - used to offer the right field
/// names on completion (an entry's `file`/`iconify`/... don't make sense
/// inside `[defaults]`, and vice versa).
pub enum SectionKind {
    TopLevel,
    Defaults,
    Link,
    Provider,
    Entry,
}

pub fn section_kind_at(text: &str, offset: usize) -> SectionKind {
    for line in text[..offset].lines().rev() {
        let trimmed = line.trim();
        let Some(inner) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
            continue;
        };
        if inner.is_empty() {
            continue;
        }
        return match inner {
            "defaults" => SectionKind::Defaults,
            "link" => SectionKind::Link,
            _ if inner.starts_with("providers") => SectionKind::Provider,
            _ => SectionKind::Entry,
        };
    }
    SectionKind::TopLevel
}

/// If `offset` lands on one of the quoted strings inside `[link]`'s
/// `includes = [...]` array, returns that (unresolved) path string.
pub fn include_target_at(text: &str, offset: usize) -> Option<String> {
    let (start, end) = section_body_range(text, "link")?;
    if offset < start || offset >= end {
        return None;
    }
    let line_start = line_start_of(text, offset);
    let line = current_line(text, offset);
    let col = offset - line_start;

    let mut search = 0;
    while let Some(rel_start) = line[search..].find('"') {
        let quote_start = search + rel_start;
        let rel_end = line[quote_start + 1..].find('"')?;
        let quote_end = quote_start + 1 + rel_end;
        if col >= quote_start && col <= quote_end + 1 {
            return Some(line[quote_start + 1..quote_end].to_string());
        }
        search = quote_end + 1;
    }
    None
}

/// The raw (unresolved) `defaults.root` string, if `[defaults]` declares
/// one - for resolving where `file`/`windows-ico` completion should look.
pub fn defaults_root(text: &str) -> Option<String> {
    let (start, end) = section_body_range(text, "defaults")?;
    for line in text[start..end].lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("root") else { continue };
        let Some(rest) = rest.trim_start().strip_prefix('=') else { continue };
        if let Some(value) = rest.trim().strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
            return Some(value.to_string());
        }
    }
    None
}

/// Which kind of path is expected for the string at `offset`, if any -
/// `file`/`windows-ico`'s single value, or one of `[link]`'s `includes`
/// array entries - used to drive filesystem-based completion.
pub enum PathFieldKind {
    File,
    Includes,
}

/// If `offset` is inside a quoted string that's a `file`/`windows-ico`
/// value or a `[link].includes` array entry, returns which kind it is,
/// the byte range of what's already been typed inside the quotes (up to
/// `offset` - the completion replacement range), and that typed prefix.
pub fn path_field_at(text: &str, offset: usize) -> Option<(PathFieldKind, std::ops::Range<usize>, String)> {
    if let Some((start, end)) = section_body_range(text, "link") {
        if offset >= start && offset < end {
            if let Some((span, prefix)) = quoted_prefix_span_at(text, offset) {
                return Some((PathFieldKind::Includes, span, prefix));
            }
        }
    }

    let line = current_line(text, offset);
    let key = line.split('=').next()?.trim();
    if key == "file" || key == "windows-ico" {
        let (span, prefix) = quoted_prefix_span_at(text, offset)?;
        return Some((PathFieldKind::File, span, prefix));
    }
    None
}

/// If `offset` is inside an `iconify = "..."` value, returns the byte
/// range of what's already been typed (up to `offset`) plus that text.
/// Purely positional - knows nothing about the `provider:name` syntax
/// inside the string, that's `iconify_completion`'s job.
pub fn iconify_field_at(text: &str, offset: usize) -> Option<(std::ops::Range<usize>, String)> {
    let line = current_line(text, offset);
    let key = line.split('=').next()?.trim();
    if key != "iconify" {
        return None;
    }
    quoted_prefix_span_at(text, offset)
}

/// If `offset` is inside a quoted string on its line, returns the byte
/// range from the opening quote up to `offset` (not the whole string -
/// only what's been typed so far) plus that same text as an owned prefix.
fn quoted_prefix_span_at(text: &str, offset: usize) -> Option<(std::ops::Range<usize>, String)> {
    let line_start = line_start_of(text, offset);
    let line = current_line(text, offset);
    let col = offset - line_start;

    let mut search = 0;
    while let Some(rel_start) = line[search..].find('"') {
        let quote_start = search + rel_start;
        let rel_end = line[quote_start + 1..].find('"')?;
        let quote_end = quote_start + 1 + rel_end;
        if col > quote_start && col <= quote_end {
            let prefix_start = line_start + quote_start + 1;
            return Some((prefix_start..offset, line[quote_start + 1..col].to_string()));
        }
        search = quote_end + 1;
    }
    None
}

/// Byte range of `[name]`'s table body (after its own header line, up to
/// the next table header or end of file).
fn section_body_range(text: &str, name: &str) -> Option<(usize, usize)> {
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
        if trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) == Some(name) {
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
    ("link", KeywordDoc {
        description: "Table declaring other manifest files to merge into this one.",
        example: "[link]\nincludes = [\"icons/nav.gui.toml\"]",
    }),
    ("includes", KeywordDoc {
        description: "Array of other manifest file paths to merge in, resolved relative to this file.",
        example: "[link]\nincludes = [\"icons/nav.gui.toml\", \"icons/toolbar.gui.toml\"]",
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

/// Byte range (absolute) of the word-forming characters immediately
/// preceding `offset` on its line - the partial key already typed.
/// Completion uses this as the exact replacement range instead of letting
/// the client guess a word boundary, which can otherwise reach back into
/// the previous line's trailing newline when `offset` is at column 0.
pub fn word_prefix_span(text: &str, offset: usize) -> std::ops::Range<usize> {
    let is_word_char = |c: char| c.is_alphanumeric() || c == '-' || c == '_';
    let line_start = line_start_of(text, offset);
    let start = text[line_start..offset]
        .rfind(|c: char| !is_word_char(c))
        .map(|i| line_start + i + 1)
        .unwrap_or(line_start);
    start..offset
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
