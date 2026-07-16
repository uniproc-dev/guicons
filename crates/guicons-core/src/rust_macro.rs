//! Detects guicons `icon!`/`icon_key!`/`icon_data!` macro-call sites in
//! `.rs` source text via a small Rust-aware scanner - not a naive
//! text/regex match (which would false-positive on "icon!" appearing
//! inside a string or comment elsewhere in the file), and not a full
//! parser either (`syn` requires the whole file to be syntactically
//! valid, which it very often isn't mid-edit). This only tracks lexical
//! state - string/comment boundaries and delimiter depth - which is
//! exactly what's needed to find a call site's argument text reliably,
//! without caring whether the rest of the file currently compiles.
//!
//! The grammar this targets is deliberately narrow: the macros only ever
//! accept a dotted path (`family.variant`) or a single string literal
//! (`"set:name"`), optionally followed by `, module = ident` - never an
//! arbitrary Rust expression - so a full AST isn't needed to interpret it
//! either (see [`crate::selector`], which does that part).
//!
//! Lives in `guicons-core` (not `guicons-lsp`, where it originated) so
//! non-LSP consumers - the `guicons-ffi` UniFFI bindings, for the IDEA
//! plugin - can call the exact same, already-tested logic instead of
//! reimplementing it in another language.

use std::ops::Range;

/// Which of the three guicons macros a call site invokes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MacroKind {
    Icon,
    IconKey,
    IconData,
}

impl MacroKind {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "icon" => Some(Self::Icon),
            "icon_key" => Some(Self::IconKey),
            "icon_data" => Some(Self::IconData),
            _ => None,
        }
    }
}

/// A located `icon!`/`icon_key!`/`icon_data!` call: which macro, and the
/// raw text + byte range of its argument token-tree (the contents of the
/// `(...)`/`[...]`/`{...}`, exclusive of the delimiters themselves).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MacroCallSite {
    pub kind: MacroKind,
    pub arg_text: String,
    pub arg_range: Range<usize>,
}

/// Finds the guicons macro call (if any) whose argument range contains
/// `offset` - used by hover to answer "what call site is my cursor
/// inside". Boundary-inclusive (`offset == arg_range.end` also counts,
/// e.g. cursor right before the closing delimiter), matching how the
/// TOML-side helpers in `guicons-lsp`'s `manifest_text.rs` treat span
/// boundaries.
pub fn macro_call_at(text: &str, offset: usize) -> Option<MacroCallSite> {
    all_macro_calls(text)
        .into_iter()
        .find(|site| offset >= site.arg_range.start && offset <= site.arg_range.end)
}

/// Finds every guicons macro call in `text`, in source order.
pub fn all_macro_calls(text: &str) -> Vec<MacroCallSite> {
    let mut sites = Vec::new();
    let len = text.len();
    let mut i = 0;
    while i < len {
        let ch = match text[i..].chars().next() {
            Some(c) => c,
            None => break,
        };
        if text[i..].starts_with("//") {
            i = skip_line_comment(text, i);
            continue;
        }
        if text[i..].starts_with("/*") {
            i = skip_block_comment(text, i);
            continue;
        }
        if ch == '"' {
            i = skip_string(text, i);
            continue;
        }
        if is_ident_start(ch) {
            let ident_start = i;
            let ident_end = scan_ident_end(text, i);
            let name = &text[ident_start..ident_end];

            let bang_pos = skip_whitespace(text, ident_end);
            if text[bang_pos..].starts_with('!') {
                let after_bang = skip_whitespace(text, bang_pos + 1);
                if let Some(open) = text[after_bang..].chars().next().filter(|c| matches!(c, '(' | '[' | '{')) {
                    if let Some(kind) = MacroKind::from_name(name) {
                        if let Some(close_byte) = find_matching_close(text, after_bang) {
                            let arg_start = after_bang + open.len_utf8();
                            sites.push(MacroCallSite {
                                kind,
                                arg_text: text[arg_start..close_byte].to_string(),
                                arg_range: arg_start..close_byte,
                            });
                            i = close_byte + 1;
                            continue;
                        }
                    }
                }
            }
            i = ident_end;
            continue;
        }
        i += ch.len_utf8();
    }
    sites
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_ascii_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_ascii_alphanumeric()
}

/// Byte offset just past the identifier starting at `start` (which must
/// already be an ident-start char). Because this always consumes a whole
/// identifier in one pass, a later comparison like `name == "icon"` can
/// never accidentally match a suffix of a longer identifier (e.g.
/// `my_icon`) - the scan would have already consumed `my_icon` as one
/// token when it first reached `m`.
fn scan_ident_end(text: &str, start: usize) -> usize {
    let mut end = start;
    for (offset, c) in text[start..].char_indices() {
        if offset == 0 || is_ident_continue(c) {
            end = start + offset + c.len_utf8();
        } else {
            break;
        }
    }
    end
}

fn skip_whitespace(text: &str, start: usize) -> usize {
    let mut i = start;
    for c in text[start..].chars() {
        if c.is_whitespace() {
            i += c.len_utf8();
        } else {
            break;
        }
    }
    i
}

fn skip_line_comment(text: &str, start: usize) -> usize {
    match text[start..].find('\n') {
        Some(rel) => start + rel + 1,
        None => text.len(),
    }
}

/// Handles `/* /* nested */ */` block comments, same as real Rust.
fn skip_block_comment(text: &str, start: usize) -> usize {
    let mut depth = 1usize;
    let mut i = start + 2;
    while i < text.len() && depth > 0 {
        if text[i..].starts_with("/*") {
            depth += 1;
            i += 2;
        } else if text[i..].starts_with("*/") {
            depth -= 1;
            i += 2;
        } else {
            i += text[i..].chars().next().map(char::len_utf8).unwrap_or(1);
        }
    }
    i
}

/// `text[start]` must be `"`. Respects `\`-escaping so an escaped `\"`
/// doesn't end the string early.
fn skip_string(text: &str, start: usize) -> usize {
    let mut i = start + 1;
    while i < text.len() {
        let c = match text[i..].chars().next() {
            Some(c) => c,
            None => break,
        };
        if c == '\\' {
            i += 1;
            if let Some(escaped) = text[i..].chars().next() {
                i += escaped.len_utf8();
            }
            continue;
        }
        if c == '"' {
            return i + 1;
        }
        i += c.len_utf8();
    }
    i
}

/// Finds the delimiter matching the open bracket at `open_byte`, tracking
/// nested delimiters of the *same kind* only (the argument grammar never
/// actually contains brackets at all, so this just needs to survive a
/// stray unrelated `}`/`)`/`]` elsewhere without ending the call early -
/// e.g. `icon!(docker.` sitting inside an existing, still-open function
/// body: the function's own closing `}` must not be mistaken for the
/// macro call's closing `)`), skipping over strings/comments along the
/// way, the same way the outer scan does. `None` if the file ends before
/// a match is found (e.g. mid-edit, closing delimiter not typed yet).
fn find_matching_close(text: &str, open_byte: usize) -> Option<usize> {
    let open = text[open_byte..].chars().next()?;
    let close = match open {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        _ => return None,
    };
    let mut depth = 1usize;
    let mut i = open_byte + open.len_utf8();
    while i < text.len() {
        if text[i..].starts_with("//") {
            i = skip_line_comment(text, i);
            continue;
        }
        if text[i..].starts_with("/*") {
            i = skip_block_comment(text, i);
            continue;
        }
        let c = text[i..].chars().next()?;
        if c == '"' {
            i = skip_string(text, i);
            continue;
        }
        if c == open {
            depth += 1;
            i += c.len_utf8();
            continue;
        }
        if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
            i += c.len_utf8();
            continue;
        }
        i += c.len_utf8();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_dotted_path_call() {
        let text = "fn f() { let x = icon!(docker.filled); }";
        let offset = text.find("docker.filled").unwrap();
        let site = macro_call_at(text, offset).expect("a call site");
        assert_eq!(site.kind, MacroKind::Icon);
        assert_eq!(site.arg_text, "docker.filled");
    }

    #[test]
    fn finds_a_string_literal_call() {
        let text = "let x = icon_key!(\"set:name\");";
        let offset = text.find("set:name").unwrap();
        let site = macro_call_at(text, offset).expect("a call site");
        assert_eq!(site.kind, MacroKind::IconKey);
        assert_eq!(site.arg_text, "\"set:name\"");
    }

    #[test]
    fn finds_icon_data_with_a_trailing_module_clause() {
        let text = "let x = icon_data!(docker.filled, module = icons2);";
        let offset = text.find("docker.filled").unwrap();
        let site = macro_call_at(text, offset).expect("a call site");
        assert_eq!(site.kind, MacroKind::IconData);
        assert_eq!(site.arg_text, "docker.filled, module = icons2");
    }

    /// The key claim: a syntax error *elsewhere* in the file - completely
    /// unrelated to the macro call - doesn't prevent finding it. A `syn`-
    /// based full parse would fail outright here.
    #[test]
    fn finds_a_call_despite_broken_surrounding_code() {
        let text = "fn broken( {\n    icon!(docker.filled)\n}\nfn also_broken syntax error here {}";
        let offset = text.find("docker.filled").unwrap();
        let site = macro_call_at(text, offset).expect("a call site");
        assert_eq!(site.arg_text, "docker.filled");
    }

    /// A call site that's itself mid-edit (no closing paren yet, the
    /// moment-to-moment state while a user is typing) must not panic, and
    /// correctly reports no match rather than a bogus partial one.
    #[test]
    fn a_call_missing_its_closing_delimiter_is_not_found() {
        let text = "fn f() {\n    icon!(docker.\n}\n";
        let offset = text.find("docker.").unwrap();
        assert_eq!(macro_call_at(text, offset), None);
        // Also must not panic when scanning the whole file for it.
        assert!(all_macro_calls(text).is_empty());
    }

    #[test]
    fn a_call_named_after_a_different_macro_is_ignored() {
        let text = "let x = println!(\"not an icon\");";
        let offset = text.find("not an icon").unwrap();
        assert_eq!(macro_call_at(text, offset), None);
    }

    /// The property that actually distinguishes this from a "regex in
    /// disguise" like I18n Ally's config-driven text match: `icon!(...)`
    /// appearing inside a string literal or a comment must never count as
    /// a real call, because the scanner tracks lexical state rather than
    /// searching the raw text.
    #[test]
    fn a_call_shaped_string_inside_a_string_literal_is_ignored() {
        let text = "let s = \"icon!(docker.filled)\";";
        assert!(all_macro_calls(text).is_empty());
    }

    #[test]
    fn a_call_shaped_comment_is_ignored() {
        let text = "// icon!(docker.filled)\nfn f() {}";
        assert!(all_macro_calls(text).is_empty());
    }

    #[test]
    fn a_call_shaped_block_comment_is_ignored() {
        let text = "/* icon!(docker.filled) */\nfn f() {}";
        assert!(all_macro_calls(text).is_empty());
    }

    /// A real call sitting right after a nested block comment must still
    /// be found - proves the nesting-depth tracking in
    /// `skip_block_comment` doesn't end the comment early at the first
    /// `*/`, which would otherwise misinterpret the rest of the comment's
    /// text as real code.
    #[test]
    fn a_nested_block_comment_does_not_cause_a_false_match_or_hide_a_real_call() {
        let text = "/* outer /* inner icon!(fake.one) */ still comment */ icon!(docker.filled)";
        let sites = all_macro_calls(text);
        assert_eq!(sites.len(), 1, "{sites:?}");
        assert_eq!(sites[0].arg_text, "docker.filled");
    }

    #[test]
    fn finds_multiple_calls_in_one_file() {
        let text = "icon!(a.one); icon_key!(b.two); icon_data!(c.three);";
        let sites = all_macro_calls(text);
        assert_eq!(sites.len(), 3);
        assert_eq!(sites[0].kind, MacroKind::Icon);
        assert_eq!(sites[1].kind, MacroKind::IconKey);
        assert_eq!(sites[2].kind, MacroKind::IconData);
    }

    #[test]
    fn cursor_exactly_at_the_argument_boundaries_still_matches() {
        let text = "icon!(a.one)";
        let start = text.find("a.one").unwrap();
        let end = start + "a.one".len();
        assert!(macro_call_at(text, start).is_some());
        assert!(macro_call_at(text, end).is_some());
        assert!(macro_call_at(text, start - 1).is_none());
        assert!(macro_call_at(text, end + 1).is_none());
    }

    #[test]
    fn a_non_ascii_comment_does_not_confuse_the_byte_scanner() {
        let text = "// non-ascii comment about icon!(fake.one)\nicon!(docker.filled)";
        let sites = all_macro_calls(text);
        assert_eq!(sites.len(), 1, "{sites:?}");
        assert_eq!(sites[0].arg_text, "docker.filled");
    }
}
