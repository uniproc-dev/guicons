use proc_macro::TokenStream;
use proc_macro2::{Ident, Span};
use quote::quote;
use std::path::PathBuf;
use syn::parse::{Parse, ParseStream};
use syn::{parse_macro_input, Error, LitInt, LitStr, Result, Token};
use winnow::ascii::alphanumeric1;
use winnow::combinator::{opt, preceded, repeat};
use winnow::token::{literal, one_of};
use winnow::{Parser, Result as WinnowResult};

/// Resolves a selector to raw icon data (`IconData`) at compile time, no
/// manifest-key indirection - the "just give me the bytes" path.
#[proc_macro]
pub fn icon(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as IconMacroInput);
    expand_icon(input)
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

/// A selector shared by `icon!` and `icon_key!`:
/// `family`/`family.variant`/`family.size.variant`/`family.size` resolves
/// against `icons.gui.toml` (the `size` segment is only needed when a
/// family has the same variant at more than one size - otherwise it's
/// redundant and can be left off); `"set:name"` (a raw iconify id)
/// resolves through `guicons-net`'s cache with no manifest lookup.
/// `icon!` turns either into `IconData`; `icon_key!` only supports
/// `FamilyVariant` - there's no manifest key for a raw iconify id.
#[derive(Clone, Debug)]
enum IconSelector {
    FamilyVariant {
        family: String,
        size: Option<u16>,
        variant: Option<String>,
    },
    Iconify(String),
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

fn expand_icon(input: IconMacroInput) -> Result<proc_macro2::TokenStream> {
    match input.selector {
        IconSelector::FamilyVariant { family, size, variant } => {
            expand_family_variant_data(&family, size, variant.as_deref())
        }
        IconSelector::Iconify(id) => expand_iconify_literal(&id),
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

fn expand_family_variant_data(
    family: &str,
    size: Option<u16>,
    variant: Option<&str>,
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

    match entry.source() {
        guicons_core::IconEntrySource::File(path) => {
            let variant_ident = Ident::new(image_kind(path), Span::call_site());
            let path = path.to_string_lossy().into_owned();
            Ok(quote! { guicons::IconData::#variant_ident(include_bytes!(#path)) })
        }
        guicons_core::IconEntrySource::Iconify(id) => expand_iconify_literal(id),
        guicons_core::IconEntrySource::Url(url) => expand_url_literal(url),
        guicons_core::IconEntrySource::Glyph(spec) => {
            let (font_family, codepoint) = guicons_core::parse_glyph_spec(spec, entry.key());
            Ok(quote! { guicons::IconData::Glyph { codepoint: #codepoint, font_family: #font_family } })
        }
    }
}

fn image_kind(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("png") => "Png",
        _ => "Svg",
    }
}

fn expand_url_literal(url: &str) -> Result<proc_macro2::TokenStream> {
    let start = manifest_dir()?;
    let cache_path = guicons_net::url_cache_path(&start, url);
    guicons_net::ensure_cached(&cache_path, url);
    let path = cache_path.to_string_lossy().into_owned();
    Ok(quote! { guicons::IconData::Svg(include_bytes!(#path)) })
}

fn expand_iconify_literal(id: &str) -> Result<proc_macro2::TokenStream> {
    let start = manifest_dir()?;
    let cache_path = guicons_net::iconify_cache_path(&start, id);
    guicons_net::ensure_cached(&cache_path, &guicons_net::iconify_url(id));
    let path = cache_path.to_string_lossy().into_owned();
    Ok(quote! { guicons::IconData::Svg(include_bytes!(#path)) })
}

fn parse_selector_literal(literal: &LitStr) -> Result<IconSelector> {
    let value = literal.value();
    if value.contains(':') {
        return Ok(IconSelector::Iconify(value));
    }
    parse_resource_selector(&value).map_err(|message| Error::new_spanned(literal, message))
}

/// One dot-separated segment of the path form (`family.24.filled`). `24`
/// lexes as a `LitInt`, not an `Ident`, so this can't just be
/// `Punctuated<Ident, Token![.]>` - each segment is read as either.
enum PathSegment {
    Ident(String),
    Size(u16),
}

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
    classify_segments(segments)
}

/// Interprets `[family]` / `[family, variant]` / `[family, size]` /
/// `[family, size, variant]` - a `Size` segment always comes before an
/// `Ident` (variant) segment, matching how `default_iconify_id` builds
/// `family-size-variant`.
fn classify_segments(segments: Vec<PathSegment>) -> Result<IconSelector> {
    let mut iter = segments.into_iter();
    let Some(PathSegment::Ident(family)) = iter.next() else {
        return Err(Error::new(
            Span::call_site(),
            "expected a family name, e.g. `settings` or `settings.filled`",
        ));
    };

    let mut size = None;
    let mut variant = None;
    for segment in iter {
        match segment {
            PathSegment::Size(value) if size.is_none() && variant.is_none() => size = Some(value),
            PathSegment::Ident(name) if variant.is_none() => variant = Some(name),
            _ => {
                return Err(Error::new(
                    Span::call_site(),
                    "expected `family`, `family.variant`, `family.size`, or `family.size.variant`",
                ));
            }
        }
    }

    Ok(IconSelector::FamilyVariant { family, size, variant })
}

fn parse_resource_selector(input: &str) -> std::result::Result<IconSelector, String> {
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
