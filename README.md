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
    guicons::build::IconBuild::auto()
        .emit_rust_registry(std::env::var("OUT_DIR").unwrap() + "/icons.rs")
        .emit_slint_global(std::env::var("OUT_DIR").unwrap() + "/icons.slint")
        .run();
}
```

```rust
// lib.rs or main.rs
mod icons {
    include!(concat!(env!("OUT_DIR"), "/icons.rs"));
}

let key = icons::key_from_dynamic_family_variant("settings", Some("filled"));
```

Use the crate normally at runtime, and enable `features = ["build"]` in
`build-dependencies` when generating code from `build.rs`.

