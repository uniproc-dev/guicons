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

let key = icons::key_from_dynamic_family_variant("settings", None, Some("filled"));
```

With the `macros` feature, `guicons::icon!` resolves a selector straight to
`IconData` at compile time (no registry lookup needed):

```rust
let data = guicons::icon!(settings.filled); // IconData::Svg(..)
```

Use `guicons::icon_key!` instead when you need a runtime-swappable
`IconKey` (theming, hot-reload via `IconResolver`):

```rust
let key = guicons::icon_key!(settings.filled); // icons::keys::SETTINGS_FILLED
```

Use `guicons` normally at runtime, and add `guicons-build` as a
`build-dependencies` entry when generating code from `build.rs`.

