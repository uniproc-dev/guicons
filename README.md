# guicons

<img src="assets/guicons-logo.svg" alt="guicons logo" width="200"/>

A manifest-driven icon system for native Rust GUI applications: describe
your icon set once in TOML (local files, URLs, iconify.design ids, font
glyphs, mixed freely), get a typed Rust registry, a Slint component, and
a compile-time-checked `icon!` macro that hands your GUI framework its
native image type directly.

## Why guicons?

Most native Rust GUI apps hardcode `include_bytes!` per icon by hand - no
manifest, no shared registry, no compile-time check that a referenced
icon actually exists in the set. guicons gives you that single source of
truth, with codegen and a macro on top so referencing an icon is a
typed, checked operation instead of a bare file path.

See [Alternatives](#alternatives) for how this compares to icon-font
crates and the `iconify` crate.

## Features

- **One manifest, `icons.gui.toml`**: family/variant/size axes,
  `[link] includes = [...]` to split across files, `[defaults]` for shared root/
  provider/size, `[providers.<name>]` schemas (built-in for Fluent,
  Phosphor, Material Symbols, Heroicons, Bootstrap Icons, Tabler, with
  per-field `.override`).
- **Any source, mixed freely**: local file, URL, iconify.design id
  (auto-fetched and cached offline), or a font glyph
  (`font-family:codepoint`).
- **Typed build-time codegen** (`guicons-build`): a Rust registry with
  per-family/size builder methods, and a matching Slint `Icon`
  component - no cartesian product of nonexistent combinations.
- **`guicons::icon!`**: resolves a selector against your manifest at
  compile time straight into your active GUI framework's native type
  (`slint::Image`, an iced `Handle`) - no `image_from_data` wrapping at
  the call site. `icon_key!` resolves to the manifest's `IconKey`
  constant instead; `icon_data!` always resolves to the plain,
  framework-agnostic `IconData`.
- **Slint integration out of the box**, with a runnable example in
  `crates/guicons/examples/`.
- **`icons` CLI** (`guicons-cli`): `icons fetch`/`update` to populate the
  offline cache, `icons add <iconify-id|file>` to add an icon with one
  command - it reverse-parses a pasted iconify id (e.g.
  `fluent:settings-24-regular`) into family/size/variant using the
  provider schema, and writes it into the manifest round-trip,
  preserving the rest of the file's formatting/comments.

## Alternatives

- **Icon-font crates** (`iced_fontello`, `egui_material_icons`,
  `iconflow`, `free-icons`, `icondata`): bundle someone else's whole
  public icon pack (as a font or SVG data), not a curated set from your
  own mix of sources. Good fit if you just want "give me Bootstrap
  Icons"; not if you have your own custom icons plus a few from a
  provider.
- **[`iconify`](https://docs.rs/iconify)**: closest to guicons'
  iconify-fetch piece alone - compile-time download, cache, and embed of
  a single id via `iconify::svg!("mdi:home")`. No manifest, no family/
  variant/size model, no local-file/URL/glyph sources, and it hands back
  a raw SVG string rather than converting to your GUI framework's native
  image type.
- **Hand-rolled `include_bytes!`**: what most apps actually do, and a
  file-path typo is already a compile error either way. What it doesn't
  give you: one manifest listing every icon your app uses instead of
  paths scattered across the codebase, iconify/URL fetching with an
  offline cache (`include_bytes!` can only embed a file you already have
  on disk), or generated per-family/variant/size builder methods instead
  of hand-writing a constant per icon.

## Usage

```toml
# icons.gui.toml
[defaults]
root = "assets/icons"

[settings]
variants.filled = { file = "settings-filled.svg" }
variants.regular = { file = "settings-regular.svg" }

[docker]
file = "docker.svg"
```

```rust
// build.rs
fn main() {
    guicons_build::IconBuild::auto()
        .emit(guicons_build::Emit::Rust)
        .emit(guicons_build::Emit::Slint)
        .build();
}
```

```rust
// lib.rs or main.rs
guicons::include_icons!();
```

Most of the time you just want to hand an icon straight to your GUI
framework - `guicons::icon!` resolves a selector, checked against your
manifest at compile time, directly to whichever framework's native type
matches your enabled feature (`slint` or `iced`; plain `IconData` if
neither is enabled):

```rust
// with the `slint` feature enabled, this is already a `slint::Image`
my_component.set_icon(guicons::icon!(settings.filled));
```

Use `guicons::icon_data!` instead if you always want the plain `IconData`,
regardless of which GUI feature is enabled.

Use `guicons` normally at runtime, and add `guicons-build` as a
`build-dependencies` entry when generating code from `build.rs`.

### Dynamic/runtime lookups

For the less common case where the family/variant is only known at
runtime (not a literal you can pass to `icon!`), or you need a
runtime-swappable `IconKey` (theming, hot-reload via `IconResolver`),
use the generated registry functions or `guicons::icon_key!` directly:

```rust
let key = icons::key_from_dynamic_family_variant("settings", None, Some("filled"));
let key = guicons::icon_key!(settings.filled); // icons::keys::SETTINGS_FILLED
```

## GUI framework integration

### Slint (`slint` feature)

The generated `icons.slint` (from `Emit::Slint` above) exports an `Icon`
component that switches on a `name` property, plus one typed component
per icon (`settings-filled` → `SettingsFilledIcon`) - `import` either
from your own `.slint` files:

```slint
import { Icon, SettingsFilledIcon } from "icons.slint"; // resolved via an include path pointing at OUT_DIR

// Dynamic - runtime string match against `name`
Icon {
    name: "settings-filled";
}

// Typed - one exported component per icon, checked at compile time
SettingsFilledIcon {}
```

For icon data you don't already have as `icon!` output - e.g. resolved at
runtime through an `IconResolver` - `guicons::slint` converts `IconData`/
`IconSource` to a Slint `Image`, or to a `(font_family, char)` pair for
glyph sources. See [`examples/slint_icon.rs`](crates/guicons/examples/slint_icon.rs)
for a complete, runnable example (`cargo run -p guicons --example slint_icon --features slint`).

### iced (`iced` feature)

`guicons::iced` gives you `iced::widget::svg::Handle`/`image::Handle`
directly from `IconData`/`IconSource`, plus the same glyph pair for
font-based icons. See [`examples/iced_icon.rs`](crates/guicons/examples/iced_icon.rs)
for a complete, runnable example (`cargo run -p guicons --example iced_icon --features iced`).

