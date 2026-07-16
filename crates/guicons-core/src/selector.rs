//! Shared interpretation of an icon "selector" - the argument to
//! `guicons::icon!`/`icon_key!`/`icon_data!` - reused by two very
//! different producers of the raw text: `guicons-macros`' `syn`-based
//! macro parser (fed a `proc_macro2::TokenStream`, already tokenized into
//! idents/int-literals/a `LitStr` by `syn` itself) and `guicons-lsp`'s
//! Rust-aware scanner (fed plain source text - see its
//! `rust_macro_detection` module). Keeping the *interpretation* of the
//! grammar (this module) separate from *tokenizing* (owned by each
//! producer, since a `syn::ParseStream` and a `&str` need very different
//! tokenizing code) means both share one definition instead of drifting
//! apart into two slightly-different parsers for the same syntax.

use winnow::ascii::alphanumeric1;
use winnow::combinator::{opt, preceded, repeat};
use winnow::token::{literal, one_of};
use winnow::{Parser, Result as WinnowResult};

/// A selector shared by `icon!` and `icon_key!`:
/// `family`/`family.variant`/`family.size.variant`/`family.size` resolves
/// against `icons.gui.toml` (the `size` segment is only needed when a
/// family has the same variant at more than one size - otherwise it's
/// redundant and can be left off); `"set:name"` (a raw iconify id)
/// resolves through `guicons-net`'s cache with no manifest lookup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IconSelector {
    FamilyVariant {
        family: String,
        size: Option<u16>,
        variant: Option<String>,
    },
    Iconify(String),
}

/// One dot-separated segment of the path form (`family.24.filled`) - `24`
/// is a size, everything else is an ident. Shared shape between the
/// `syn`-token-driven path parser (`guicons-macros`, which reads a
/// `syn::LitInt`/`syn::Ident` directly off a `ParseStream`) and the
/// plain-text one below (which just splits on `.`) - each only differs in
/// how it tokenizes into this shape, not in what the shape means.
#[derive(Clone, Debug)]
pub enum PathSegment {
    Ident(String),
    Size(u16),
}

/// Interprets `[family]` / `[family, variant]` / `[family, size]` /
/// `[family, size, variant]` - a `Size` segment always comes before an
/// `Ident` (variant) segment, matching how `default_iconify_id` builds
/// `family-size-variant`.
pub fn classify_segments(segments: Vec<PathSegment>) -> Result<IconSelector, String> {
    let mut iter = segments.into_iter();
    let Some(PathSegment::Ident(family)) = iter.next() else {
        return Err("expected a family name, e.g. `settings` or `settings.filled`".to_string());
    };

    let mut size = None;
    let mut variant = None;
    for segment in iter {
        match segment {
            PathSegment::Size(value) if size.is_none() && variant.is_none() => size = Some(value),
            PathSegment::Ident(name) if variant.is_none() => variant = Some(name),
            _ => {
                return Err("expected `family`, `family.variant`, `family.size`, or `family.size.variant`".to_string());
            }
        }
    }

    Ok(IconSelector::FamilyVariant { family, size, variant })
}

/// Plain-text version of the dotted-path form (`family`, `family.variant`,
/// `family.size`, `family.size.variant`) - splits on `.` and classifies
/// each segment as a size (parses as `u16`) or an ident, then delegates to
/// the same [`classify_segments`] the `syn`-token-driven parser in
/// `guicons-macros` uses. `_` is normalized to `-` per segment (matching
/// the existing convention the `syn`-token parser already applies), since
/// this is fed plain identifier text straight from Rust source, not a
/// `syn::Ident` with its own normalization step.
pub fn parse_selector_path_text(input: &str) -> Result<IconSelector, String> {
    if input.is_empty() {
        return Err("expected a family name, e.g. `settings` or `settings.filled`".to_string());
    }
    let segments = input
        .split('.')
        .map(|segment| match segment.parse::<u16>() {
            Ok(size) => PathSegment::Size(size),
            Err(_) => PathSegment::Ident(segment.replace('_', "-")),
        })
        .collect();
    classify_segments(segments)
}

/// Parses the string-literal form: `"family/variant"`, `"family/size/variant"`,
/// or a raw iconify id `"set:name"` (checked first via `contains(':')`,
/// since `:` never appears in the slash-separated form and `/` never
/// appears in an iconify id).
pub fn parse_resource_selector(input: &str) -> Result<IconSelector, String> {
    if input.contains(':') {
        return Ok(IconSelector::Iconify(input.to_string()));
    }

    let mut parser = (
        resource_segment,
        opt(preceded(literal("/"), resource_segment)),
        opt(preceded(literal("/"), resource_segment)),
    );
    let mut input_rest = input;
    let (family, second, third) = parser.parse_next(&mut input_rest).map_err(|_| {
        "expected icon selector like `settings`, `settings/filled`, or `settings/24/filled`".to_string()
    })?;
    if !input_rest.is_empty() {
        return Err(format!("unexpected trailing input `{input_rest}` in icon selector"));
    }

    let (size, variant) = match (second, third) {
        (None, None) => (None, None),
        (Some(only), None) => match only.parse::<u16>() {
            Ok(size) => (Some(size), None),
            Err(_) => (None, Some(only)),
        },
        (Some(size_segment), Some(variant)) => {
            let size = size_segment
                .parse::<u16>()
                .map_err(|_| format!("expected a numeric size before the variant, got `{size_segment}`"))?;
            (Some(size), Some(variant))
        }
        (None, Some(_)) => unreachable!("winnow's `opt` chain can't produce a third segment without a second"),
    };

    Ok(IconSelector::FamilyVariant { family, size, variant })
}

fn resource_segment(input: &mut &str) -> WinnowResult<String> {
    (
        alphanumeric1,
        repeat::<_, _, (), _, _>(0.., (one_of(['-', '_']), alphanumeric1)),
    )
        .take()
        .map(str::to_string)
        .parse_next(input)
}

/// The single shared entry point for interpreting a selector from *raw,
/// untokenized* argument text - used only by `guicons-lsp` (which only has
/// plain source text to work with, unlike `guicons-macros`, which already
/// knows definitively whether it's looking at a `syn::LitStr` or a bare
/// path before it ever needs to interpret one). Trims a trailing
/// `, module = ident` if present (irrelevant to resolving the icon itself
/// - hover doesn't need to know which module the key would land in),
/// then dispatches on whether what's left is a quoted string literal or a
/// bare dotted path.
pub fn parse_selector(raw: &str) -> Result<IconSelector, String> {
    let trimmed = raw.trim();
    let (selector_part, _module_part) = split_off_module(trimmed);
    let selector_part = selector_part.trim();

    if let Some(literal) = selector_part.strip_prefix('"').and_then(|rest| rest.strip_suffix('"')) {
        return parse_resource_selector(literal);
    }
    parse_selector_path_text(selector_part)
}

/// Splits `text` on its first top-level `,` - selectors themselves never
/// contain a comma (the only place one can appear in the macro's
/// grammar is right before `module = ident`), so this doesn't need to be
/// comma-inside-a-string-aware the way a general Rust tokenizer would.
fn split_off_module(text: &str) -> (&str, Option<&str>) {
    match text.split_once(',') {
        Some((before, after)) => (before, Some(after)),
        None => (text, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_bare_family() {
        assert_eq!(
            parse_selector_path_text("settings").unwrap(),
            IconSelector::FamilyVariant { family: "settings".to_string(), size: None, variant: None }
        );
    }

    #[test]
    fn parses_a_family_and_variant() {
        assert_eq!(
            parse_selector_path_text("settings.filled").unwrap(),
            IconSelector::FamilyVariant {
                family: "settings".to_string(),
                size: None,
                variant: Some("filled".to_string())
            }
        );
    }

    #[test]
    fn parses_a_family_size_and_variant() {
        assert_eq!(
            parse_selector_path_text("settings.24.filled").unwrap(),
            IconSelector::FamilyVariant {
                family: "settings".to_string(),
                size: Some(24),
                variant: Some("filled".to_string())
            }
        );
    }

    #[test]
    fn underscore_in_a_path_segment_normalizes_to_a_dash() {
        assert_eq!(
            parse_selector_path_text("nav_bar.filled").unwrap(),
            IconSelector::FamilyVariant {
                family: "nav-bar".to_string(),
                size: None,
                variant: Some("filled".to_string())
            }
        );
    }

    #[test]
    fn a_size_after_a_variant_is_rejected() {
        assert!(parse_selector_path_text("settings.filled.24").is_err());
    }

    #[test]
    fn empty_input_is_rejected() {
        assert!(parse_selector_path_text("").is_err());
    }

    #[test]
    fn parses_a_slash_separated_family_and_variant() {
        assert_eq!(
            parse_resource_selector("settings/filled").unwrap(),
            IconSelector::FamilyVariant {
                family: "settings".to_string(),
                size: None,
                variant: Some("filled".to_string())
            }
        );
    }

    #[test]
    fn parses_a_slash_separated_family_size_and_variant() {
        assert_eq!(
            parse_resource_selector("settings/24/filled").unwrap(),
            IconSelector::FamilyVariant {
                family: "settings".to_string(),
                size: Some(24),
                variant: Some("filled".to_string())
            }
        );
    }

    #[test]
    fn a_colon_makes_it_an_iconify_id_regardless_of_slashes() {
        assert_eq!(parse_resource_selector("mdi:home").unwrap(), IconSelector::Iconify("mdi:home".to_string()));
    }

    #[test]
    fn parse_selector_dispatches_a_quoted_string_to_the_resource_parser() {
        assert_eq!(
            parse_selector("\"mdi:home\"").unwrap(),
            IconSelector::Iconify("mdi:home".to_string())
        );
    }

    #[test]
    fn parse_selector_dispatches_a_bare_path_to_the_path_parser() {
        assert_eq!(
            parse_selector("settings.filled").unwrap(),
            IconSelector::FamilyVariant {
                family: "settings".to_string(),
                size: None,
                variant: Some("filled".to_string())
            }
        );
    }

    #[test]
    fn parse_selector_strips_a_trailing_module_clause() {
        assert_eq!(
            parse_selector("settings.filled, module = icons2").unwrap(),
            IconSelector::FamilyVariant {
                family: "settings".to_string(),
                size: None,
                variant: Some("filled".to_string())
            }
        );
    }
}
