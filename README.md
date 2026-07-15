# guicons

`guicons` is a manifest-driven icon system for native Rust GUI applications.
It provides a small runtime API plus optional build-time code generation for
typed icon registries and Slint components.

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
component that switches on a `name` property - `import` it from your own
`.slint` files:

```slint
import { Icon } from "icons.slint"; // resolved via an include path pointing at OUT_DIR

Icon {
    name: "settings-filled";
}
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

