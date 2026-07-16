use guicons_core::selector::{classify_segments, IconSelector, PathSegment};
use proc_macro::TokenStream;
use proc_macro2::{Ident, Span};
use quote::quote;
use std::path::PathBuf;
use syn::parse::{Parse, ParseStream};
use syn::{parse_macro_input, Error, LitInt, LitStr, Result, Token};

#[cfg(all(feature = "slint", feature = "iced"))]
compile_error!(
    "guicons-macros: enable only one of the `slint`/`iced` features at a time - `icon!` picks its \
     target automatically from whichever is active. Use `icon_data!` if you need the plain `IconData` \
     regardless of which GUI feature is enabled."
);

/// Which native type `icon!` emits, chosen automatically from whichever of
/// `guicons-macros`' `slint`/`iced` features is active (mirrored from
/// `guicons`' own features of the same name). Falls back to `IconData`
/// when neither is enabled.
#[derive(Clone, Copy)]
enum Target {
    Data,
    #[cfg(feature = "slint")]
    Slint,
    #[cfg(feature = "iced")]
    Iced,
}

fn active_target() -> Target {
    #[cfg(feature = "slint")]
    return Target::Slint;
    #[cfg(feature = "iced")]
    return Target::Iced;
    #[allow(unreachable_code)]
    Target::Data
}

/// Resolves a selector straight to the native icon type for whichever GUI
/// feature is active (`slint`/`iced`), or to `IconData` if neither is - no
/// manifest-key indirection, no wrapping call needed at the use site. Use
/// [`icon_data!`] to always get `IconData`, regardless of active features.
#[proc_macro]
pub fn icon(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as IconMacroInput);
    expand_icon(input, active_target())
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// Like [`icon!`], but always resolves to plain `IconData` - the explicit
/// escape hatch from `icon!`'s automatic native-type target.
#[proc_macro]
pub fn icon_data(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as IconMacroInput);
    expand_icon(input, Target::Data)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// Resolves a `family`/`family.variant`/`family.size`/`family.size.variant`
/// selector to the manifest's `IconKey` constant, for callers that need
/// runtime-swappable resolution (theming, hot-reload via `IconResolver`).
#[proc_macro]
pub fn icon_key(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as IconMacroInput);
    expand_icon_key(input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

struct IconMacroInput {
    selector: IconSelector,
    module: Ident,
}

impl Parse for IconMacroInput {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let selector = if input.peek(LitStr) {
            let literal: LitStr = input.parse()?;
            parse_selector_literal(&literal)?
        } else {
            parse_selector_path(input)?
        };

        let mut module = Ident::new("icons", Span::call_site());
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
            let key: Ident = input.parse()?;
            if key != "module" {
                return Err(Error::new_spanned(key, "expected `module = ...`"));
            }
            input.parse::<Token![=]>()?;
            module = input.parse()?;
        }

        if !input.is_empty() {
            return Err(input.error("unexpected tokens in guicons::icon! input"));
        }

        Ok(Self { selector, module })
    }
}

fn expand_icon(input: IconMacroInput, target: Target) -> Result<proc_macro2::TokenStream> {
    match input.selector {
        IconSelector::FamilyVariant { family, size, variant } => {
            expand_family_variant_data(&family, size, variant.as_deref(), target)
        }
        IconSelector::Iconify(id) => expand_iconify_literal(&id, target),
    }
}

fn expand_icon_key(input: IconMacroInput) -> Result<proc_macro2::TokenStream> {
    match input.selector {
        IconSelector::FamilyVariant { family, size, variant } => {
            expand_family_variant_key(&family, size, variant.as_deref(), input.module)
        }
        IconSelector::Iconify(id) => Err(Error::new(
            Span::call_site(),
            format!(
                "`icon_key!` doesn't support iconify literals - there's no manifest key for a raw iconify id `{id}`, use `icon!` instead"
            ),
        )),
    }
}

fn expand_family_variant_key(
    family: &str,
    size: Option<u16>,
    variant: Option<&str>,
    module: Ident,
) -> Result<proc_macro2::TokenStream> {
    let manifest_path = manifest_dir()?.join("icons.gui.toml");
    let manifest = load_manifest(&manifest_path)?;
    let key = match manifest.entry_for_family_variant(family, size, variant) {
        Some(entry) => entry.key().to_string(),
        None => {
            return Err(Error::new(
                Span::call_site(),
                unknown_icon_message(&manifest, family, size, variant),
            ));
        }
    };

    let key_ident = Ident::new(&guicons_core::rust_const_name(&key), Span::call_site());
    Ok(quote! { #module::keys::#key_ident })
}

/// What an entry's source resolves to, before deciding which native type
/// (or plain `IconData`) to wrap it in for the requested [`Target`].
enum ResolvedSource {
    Image { path: String, kind: &'static str },
    Glyph { font_family: String, codepoint: char },
}

fn expand_family_variant_data(
    family: &str,
    size: Option<u16>,
    variant: Option<&str>,
    target: Target,
) -> Result<proc_macro2::TokenStream> {
    let manifest_path = manifest_dir()?.join("icons.gui.toml");
    let manifest = load_manifest(&manifest_path)?;
    let entry = match manifest.entry_for_family_variant(family, size, variant) {
        Some(entry) => entry,
        None => {
            return Err(Error::new(
                Span::call_site(),
                unknown_icon_message(&manifest, family, size, variant),
            ));
        }
    };

    let resolved = match entry.source() {
        guicons_core::IconEntrySource::File(path) => ResolvedSource::Image {
            kind: image_kind(path),
            path: path.to_string_lossy().into_owned(),
        },
        guicons_core::IconEntrySource::Iconify(id) => resolve_iconify_source(id)?,
        guicons_core::IconEntrySource::Url(url) => resolve_url_source(url)?,
        guicons_core::IconEntrySource::Glyph(spec) => {
            let (font_family, codepoint) = guicons_core::parse_glyph_spec(spec, entry.key());
            ResolvedSource::Glyph { font_family, codepoint }
        }
    };

    Ok(emit_for_target(resolved, target))
}

fn image_kind(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("png") => "Png",
        _ => "Svg",
    }
}

fn resolve_url_source(url: &str) -> Result<ResolvedSource> {
    let start = manifest_dir()?;
    let cache_path = guicons_net::url_cache_path(&start, url);
    guicons_net::ensure_cached(&cache_path, url);
    Ok(ResolvedSource::Image {
        path: cache_path.to_string_lossy().into_owned(),
        kind: "Svg",
    })
}

fn resolve_iconify_source(id: &str) -> Result<ResolvedSource> {
    let start = manifest_dir()?;
    let cache_path = guicons_net::iconify_cache_path(&start, id);
    guicons_net::ensure_cached(&cache_path, &guicons_net::iconify_url(id));
    Ok(ResolvedSource::Image {
        path: cache_path.to_string_lossy().into_owned(),
        kind: "Svg",
    })
}

fn expand_iconify_literal(id: &str, target: Target) -> Result<proc_macro2::TokenStream> {
    let resolved = resolve_iconify_source(id)?;
    Ok(emit_for_target(resolved, target))
}

/// Wraps a resolved source in the token stream for `target` - plain
/// `IconData` for [`Target::Data`], or a call into `guicons::slint`/
/// `guicons::iced`'s conversion helpers for the GUI-specific targets, which
/// return `Option`/panic via `.expect(..)` since the conversion itself can
/// fail (e.g. malformed PNG bytes) - that's still a compile-time-cached
/// asset, so a panic here means the cached file is corrupt, not a user
/// input problem.
fn emit_for_target(resolved: ResolvedSource, target: Target) -> proc_macro2::TokenStream {
    let data_tokens = match &resolved {
        ResolvedSource::Image { path, kind } => {
            let kind_ident = Ident::new(kind, Span::call_site());
            quote! { guicons::IconData::#kind_ident(include_bytes!(#path)) }
        }
        ResolvedSource::Glyph { font_family, codepoint } => {
            quote! { guicons::IconData::Glyph { codepoint: #codepoint, font_family: #font_family } }
        }
    };

    match target {
        Target::Data => data_tokens,
        #[cfg(feature = "slint")]
        Target::Slint => match resolved {
            ResolvedSource::Image { .. } => quote! {
                guicons::slint::image_from_data(#data_tokens).expect("guicons: cached icon asset failed to decode")
            },
            ResolvedSource::Glyph { .. } => quote! {
                guicons::slint::glyph_from_data(#data_tokens).expect("guicons: icon entry is not a glyph")
            },
        },
        #[cfg(feature = "iced")]
        Target::Iced => match &resolved {
            ResolvedSource::Image { kind, .. } if *kind == "Png" => quote! {
                guicons::iced::image_handle_from_data(#data_tokens).expect("guicons: cached icon asset failed to decode")
            },
            ResolvedSource::Image { .. } => quote! {
                guicons::iced::svg_handle_from_data(#data_tokens).expect("guicons: cached icon asset failed to decode")
            },
            ResolvedSource::Glyph { .. } => quote! {
                guicons::iced::glyph_from_data(#data_tokens).expect("guicons: icon entry is not a glyph")
            },
        },
    }
}

/// Thin wrapper over `guicons_core::selector::parse_resource_selector` -
/// shared with `guicons-lsp`'s hover, which parses the exact same grammar
/// from plain scanned text rather than a `syn::LitStr`.
fn parse_selector_literal(literal: &LitStr) -> Result<IconSelector> {
    guicons_core::selector::parse_resource_selector(&literal.value()).map_err(|message| Error::new_spanned(literal, message))
}

/// `24` in `family.24.filled` lexes as a `LitInt`, not an `Ident`, so this
/// can't just be `Punctuated<Ident, Token![.]>` - each segment is read as
/// either, then classified by the shared `classify_segments`.
fn parse_selector_path(input: ParseStream<'_>) -> Result<IconSelector> {
    let mut segments = Vec::new();
    loop {
        if input.peek(LitInt) {
            let lit: LitInt = input.parse()?;
            let value = lit
                .base10_parse::<u16>()
                .map_err(|_| Error::new_spanned(&lit, "size must fit in a u16"))?;
            segments.push(PathSegment::Size(value));
        } else {
            let ident: Ident = input.parse()?;
            segments.push(PathSegment::Ident(ident.to_string().replace('_', "-")));
        }
        if input.peek(Token![.]) {
            input.parse::<Token![.]>()?;
        } else {
            break;
        }
    }
    classify_segments(segments).map_err(|message| Error::new(Span::call_site(), message))
}

fn manifest_dir() -> Result<PathBuf> {
    let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR")
        .ok_or_else(|| Error::new(Span::call_site(), "CARGO_MANIFEST_DIR is not set"))?;
    Ok(PathBuf::from(manifest_dir))
}

fn load_manifest(path: &std::path::Path) -> Result<guicons_core::IconManifest> {
    if !path.exists() {
        return Err(Error::new(
            Span::call_site(),
            format!("guicons manifest not found at {}", path.display()),
        ));
    }

    let (manifest, errors) = guicons_core::load_icon_manifest(path);

    if !errors.is_empty() {
        let messages = errors
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("; ");

        return Err(Error::new(
            Span::call_site(),
            format!("guicons manifest at {} has errors: {}", path.display(), messages),
        ));
    }

    Ok(manifest)
}

fn unknown_icon_message(
    manifest: &guicons_core::IconManifest,
    family: &str,
    size: Option<u16>,
    variant: Option<&str>,
) -> String {
    let mut display = family.to_string();
    if let Some(size) = size {
        display.push('/');
        display.push_str(&size.to_string());
    }
    if let Some(variant) = variant {
        display.push('/');
        display.push_str(variant);
    }

    let variants = manifest
        .entries()
        .iter()
        .filter(|entry| entry.family() == family && entry.size() == size)
        .filter_map(|entry| entry.variant())
        .collect::<Vec<_>>();

    if variants.is_empty() {
        format!("unknown guicons icon `{display}`")
    } else {
        format!(
            "unknown guicons icon `{display}`; known variants for `{family}`{}: {}",
            size.map(|size| format!("/{size}")).unwrap_or_default(),
            variants.join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse::Parser;

    /// `parse_selector_path` (this crate, token-driven off a real
    /// `syn::ParseStream`) and `guicons_core::selector::parse_selector_path_text`
    /// (`guicons-lsp`'s plain-text equivalent) must agree on every dotted-
    /// path selector - cheap insurance against the two entry points
    /// drifting apart now that they're two separate tokenizers feeding
    /// the same shared `classify_segments`.
    #[test]
    fn syn_and_text_path_parsers_agree_on_known_selectors() {
        for input in ["settings", "settings.filled", "settings.24.filled", "settings.20", "nav_bar.filled"] {
            let tokens: proc_macro2::TokenStream = input.parse().unwrap();
            let via_syn = parse_selector_path.parse2(tokens).unwrap();
            let via_text = guicons_core::selector::parse_selector_path_text(input).unwrap();
            assert_eq!(via_syn, via_text, "mismatch for `{input}`");
        }
    }
}
