use proc_macro::TokenStream;
use proc_macro2::{Ident, Span};
use quote::quote;
use std::path::PathBuf;
use syn::parse::{Parse, ParseStream};
use syn::{parse_macro_input, Error, LitStr, Result, Token};
use syn::punctuated::Punctuated;
use winnow::ascii::alphanumeric1;
use winnow::combinator::{opt, preceded, repeat};
use winnow::token::{literal, one_of};
use winnow::{Parser, Result as WinnowResult};

#[proc_macro]
pub fn icon(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as IconMacroInput);
    expand_icon(input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

struct IconMacroInput {
    selector: IconSelector,
    module: Ident,
}

/// `guicons::icon!` accepts two unrelated shapes of selector:
///
/// - `family`/`family.variant`/`"family/variant"` - resolved against
///   `icons.gui.toml`, expands to a `keys::` constant (an `IconKey`).
/// - `"set:name"` (a raw iconify id, straight off iconify.design) - resolved
///   through `guicons-net`'s cache directly, with **no manifest lookup at
///   all**, expanding to `IconData` instead. Adding the same id to the
///   manifest later doesn't change what an existing call site resolves to:
///   both paths key the on-disk cache by the exact same string.
#[derive(Clone, Debug)]
enum IconSelector {
    FamilyVariant { family: String, variant: Option<String> },
    Iconify(String),
}

impl Parse for IconMacroInput {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let selector = if input.peek(LitStr) {
            let literal: LitStr = input.parse()?;
            parse_selector_literal(&literal)?
        } else {
            let segments = Punctuated::<Ident, Token![.]>::parse_separated_nonempty(input)?;
            parse_selector_path(segments)?
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
        IconSelector::FamilyVariant { family, variant } => {
            expand_family_variant(&family, variant.as_deref(), input.module)
        }
        IconSelector::Iconify(id) => expand_iconify_literal(&id),
    }
}

fn expand_family_variant(
    family: &str,
    variant: Option<&str>,
    module: Ident,
) -> Result<proc_macro2::TokenStream> {
    let manifest_path = manifest_dir()?.join("icons.gui.toml");
    let manifest = load_manifest(&manifest_path)?;
    let key = match manifest.entry_for_family_variant(family, variant) {
        Some(entry) => entry.key().to_string(),
        None => {
            return Err(Error::new(
                Span::call_site(),
                unknown_icon_message(&manifest, family, variant),
            ));
        }
    };

    let key_ident = Ident::new(&guicons_core::rust_const_name(&key), Span::call_site());
    Ok(quote! { #module::keys::#key_ident })
}

/// Resolves a raw `"set:name"` iconify id straight through `guicons-net`'s
/// cache - no manifest, no registry, just the SVG bytes for this one icon.
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

fn parse_selector_path(segments: Punctuated<Ident, Token![.]>) -> Result<IconSelector> {
    let segments = segments.into_iter().collect::<Vec<_>>();
    match segments.as_slice() {
        [family] => Ok(IconSelector::FamilyVariant {
            family: family.to_string().replace('_', "-"),
            variant: None,
        }),
        [family, variant] => Ok(IconSelector::FamilyVariant {
            family: family.to_string().replace('_', "-"),
            variant: Some(variant.to_string().replace('_', "-")),
        }),
        _ => {
            let mut iter = segments.iter();
            let first = iter.next().unwrap();
            let span = iter.fold(first.span(), |s, seg| s.join(seg.span()).unwrap_or(s));
            Err(Error::new(
                span,
                "expected `family`, `family.variant`, or a string literal like \"family/variant\"",
            ))
        }
    }
}

fn parse_resource_selector(input: &str) -> std::result::Result<IconSelector, String> {
    let mut parser = (
        resource_segment,
        opt(preceded(literal("/"), resource_segment)),
    )
        .map(|(family, variant)| IconSelector::FamilyVariant { family, variant });
    let mut input = input;
    let selector = parser
        .parse_next(&mut input)
        .map_err(|_| "expected icon selector like `settings` or `settings/filled`".to_string())?;
    if !input.is_empty() {
        return Err(format!("unexpected trailing input `{input}` in icon selector"));
    }
    Ok(selector)
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
    variant: Option<&str>,
) -> String {
    let display = match variant {
        Some(variant) => format!("{family}/{variant}"),
        None => family.to_string(),
    };
    let variants = manifest
        .entries()
        .iter()
        .filter(|entry| entry.family() == family)
        .filter_map(|entry| entry.variant())
        .collect::<Vec<_>>();

    if variants.is_empty() {
        format!("unknown guicons icon `{display}`")
    } else {
        format!(
            "unknown guicons icon `{display}`; known variants for `{family}`: {}",
            variants.join(", ")
        )
    }
}
