# guicons-idea-plugin

Native icon preview for guicons' `icon!`/`icon_key!`/`icon_data!`
macro calls: renders the resolved SVG/PNG directly inside the Quick
Documentation popup (Ctrl+Q / F1), instead of only the text description
`guicons-lsp`'s own hover already provides.

This is the "Track B" plugin mentioned in the main repo's memory - a real
IntelliJ Platform plugin, separate from the LSP protocol, because Swing's
documentation popup can render an `<img>` where a markdown/plaintext LSP
hover can't.

## Status: compiles, otherwise unreviewed

`./gradlew compileKotlin` passes against a real local RustRover 2025.2.3 +
its bundled `com.jetbrains.rust` plugin - that much has actually been
verified, not just written and hoped for. Still not run: nobody has ever
opened Quick Documentation on a real `icon!(...)` call in a running
sandboxed IDE (`./gradlew runIde`) and looked at what comes back.

- `local("C:/Program Files/JetBrains/RustRover 2025.2.3")` in
  `build.gradle.kts` points at one specific machine's install path -
  works here, will not work anywhere else as-is. Switch to
  `rustRover("<exact build number>")` (not just `"2025.2"` - needs the
  full version JetBrains actually published, e.g. `"2025.2.3"`) to have
  Gradle fetch a matching IDE instead of relying on a local install.
- Gradle wrapper is committed now (`gradlew`/`gradlew.bat`), pinned to
  Gradle 9.0 - required by IntelliJ Platform Gradle Plugin 2.17.0.
  Building needs JDK 21 available to Gradle (`org.gradle.java.installations.auto-download`
  is on by default, so Gradle should provision one itself if none is
  registered).

## How it works

Two layers: the actual detection/parsing/resolution logic, and the
native-UI-specific rendering wrapped around it.

**Logic - not reimplemented in Kotlin.** `crates/guicons-ffi` (in the main
repo, not this directory) is a thin [UniFFI](https://mozilla.github.io/uniffi-rs/)
wrapper around `guicons-core`'s already-tested `rust_macro`/`selector`
modules - the exact same code `guicons-lsp` uses for its own `.rs`-side
hover. `src/main/kotlin/uniffi/guicons_ffi/guicons_ffi.kt` is the
UniFFI-generated Kotlin binding for it (checked in, not generated at build
time yet - see "Regenerating the bindings" below), and
`src/main/resources/win32-x86-64/guicons_ffi.dll` is the compiled native
library it calls into via JNA. This replaced an earlier version of this
plugin that hand-ported the Rust scanning/parsing logic into Kotlin
(`MacroCallDetector.kt`/`SelectorParser.kt`/`ManifestLookup.kt`, since
deleted) - two independent implementations of the same logic drifting
apart over time was exactly the failure mode worth avoiding.

**Rendering - genuinely Kotlin/Swing-specific, stays here:**

- **`SvgRenderer.kt`** - rasterizes SVG to PNG via Apache Batik before
  embedding as a base64 `<img>` data URI, since Swing's HTML renderer
  doesn't reliably support inline SVG that way (unlike PNG/JPEG, SVG isn't
  a registered `ImageIO` format) - guicons' icons are overwhelmingly SVG,
  so skipping this step would make the "native rendering" pitch mostly
  not work.
- **`GuiconsDocumentationProvider.kt`** - the `AbstractDocumentationProvider`
  registered for the Rust language in `plugin.xml`. Calls
  `macroCallAt`/`parseSelector`/`findManifestForRustFile`/
  `resolveFamilyVariant` from the generated bindings, then builds the
  HTML popup (calling `SvgRenderer` for the image).

### Regenerating the bindings

After changing `crates/guicons-ffi`'s exported API:

```powershell
cd ..\crates\guicons-ffi
cargo build --release
cargo run --release --bin uniffi-bindgen --features uniffi/cli -- generate `
    --library ..\..\target\release\guicons_ffi.dll `
    --language kotlin --out-dir generated
```

then copy `generated/uniffi/guicons_ffi/guicons_ffi.kt` over
`ide-plugin/src/main/kotlin/uniffi/guicons_ffi/guicons_ffi.kt`, and
`target/release/guicons_ffi.dll` over
`ide-plugin/src/main/resources/win32-x86-64/guicons_ffi.dll`. Not wired
into a Gradle task yet - a real next step, not just busywork, since
forgetting this step silently leaves the plugin calling a stale native
library.

Only a Windows (`win32-x86-64`) build of the native library is bundled -
no macOS/Linux `.dylib`/`.so` built or tested.

## Known gaps / next steps

- **Cross-platform native library builds.** Only Windows
  (`win32-x86-64/guicons_ffi.dll`) is built/bundled right now - JNA
  resolves the right file for the running OS/arch automatically *if it's
  present*, so Linux/macOS users currently get a load failure at runtime,
  not silent breakage, but still broken. Needs: `rustup target add` +
  `cargo build --release --target <triple>` for
  `x86_64-unknown-linux-gnu`/`aarch64-unknown-linux-gnu`/
  `x86_64-apple-darwin`/`aarch64-apple-darwin` (via `cross` or
  `cargo-zigbuild` to cross-compile without native toolchains/VMs for
  each), dropping each output into its own
  `src/main/resources/<jna-os-arch>/` folder (`linux-x86-64`,
  `linux-aarch64`, `darwin-x86-64`, `darwin-aarch64`) alongside the
  existing `win32-x86-64` one - all end up in the same plugin jar, JNA
  picks the matching one at runtime. Kotlin bindings don't change per
  target, only the native lib. Best done as a CI matrix job (one per
  target) rather than locally - real macOS output needs a real macOS
  runner, cross-compiling *to* Apple targets from Windows/Linux is
  painful.
- **No test suite on the Kotlin side at all.** The Rust side
  (`guicons-core::rust_macro`, `guicons-ffi`) has real unit tests; nothing
  here exercises `GuiconsDocumentationProvider`/`SvgRenderer` themselves,
  or the JNA/native-library loading path specifically.
- `iconify`/`url`/`glyph` sources aren't previewed at all - only `file`
  (`ResolvedEntry.sourceFile` is `None` for those; see `guicons-ffi`).
- Bindings-regeneration isn't automated (see above) - a real risk of the
  checked-in `.kt`/`.dll` silently going stale relative to
  `guicons-ffi`'s Rust source.
- Only ever run through `compileKotlin` - `./gradlew runIde` (the actual
  "does hovering an icon! call show a picture" test) has never been done.
