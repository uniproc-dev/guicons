plugins {
    id("org.jetbrains.kotlin.jvm") version "2.1.0"
    // 2.1.0 had a real "No IntelliJ Platform dependency found" resolution
    // bug (fixed by 2.2.0) - bumped straight to latest to avoid it and
    // whatever else has been fixed since. Latest requires Gradle 9+, see
    // the wrapper task below.
    id("org.jetbrains.intellij.platform") version "2.17.0"
}

group = "dev.guicons"
version = "0.1.0"

repositories {
    mavenCentral()
    intellijPlatform {
        defaultRepositories()
    }
}

dependencies {
    intellijPlatform {
        // Points at the RustRover already installed locally instead of
        // having Gradle download a whole separate IDE distribution just to
        // compile against - `local()` takes the installation directory
        // directly. Only works on this machine as-is; CI/another dev's
        // machine would need either their own local path or switching back
        // to `rustRover("<exact build>")` to fetch one.
        local("C:/Program Files/JetBrains/RustRover 2025.2.3")
        bundledPlugin("com.jetbrains.rust")
    }

    // Rasterizes SVG assets to PNG for the doc-popup preview - Swing's
    // HTML renderer doesn't reliably support inline SVG via a base64 data
    // URI (unlike PNG/JPEG, SVG isn't a registered ImageIO format), so
    // guicons' overwhelmingly-SVG icons need converting to something the
    // popup can actually display.
    //
    // `xml-apis`/`xml-apis-ext` excluded: a legacy pre-JDK-JAXP artifact
    // that bundles its own `javax.xml.parsers.*`/`javax.xml.transform.*`
    // classes - loaded by the plugin's own `PluginClassLoader` if bundled,
    // while the platform resolves its JAXP provider
    // (`org.apache.xerces.jaxp.SAXParserFactoryImpl`) through its own
    // `PathClassLoader`. Two different classloaders for what the JVM
    // treats as the same type -> `ClassCastException` at runtime. JAXP is
    // just part of the JDK now, so this is pure legacy baggage batik still
    // transitively drags in - not needed on any JVM this plugin targets.
    implementation("org.apache.xmlgraphics:batik-transcoder:1.17") {
        exclude(group = "xml-apis")
    }
    implementation("org.apache.xmlgraphics:batik-codec:1.17") {
        exclude(group = "xml-apis")
    }

    // Loads ../crates/guicons-ffi's native library - the generated UniFFI
    // Kotlin bindings (src/main/kotlin/uniffi/guicons_ffi/guicons_ffi.kt)
    // use JNA's `Native.load` to call into it. `compileOnly`, not
    // `implementation` - deliberately NOT bundled into the plugin, so at
    // runtime `PluginClassLoader` (which checks the plugin's own jars
    // before delegating to the platform) finds no local `com.sun.jna.*`
    // and falls through to the IntelliJ Platform's own bundled JNA
    // instead. That's the only version whose native dispatch lib actually
    // matches the `-Djna.boot.library.path`/`-Djna.nosys`/
    // `-Djna.noclasspath` the platform sets up for itself - bundling our
    // own JNA jar here shadows it with a version whose native ABI won't
    // match, and fails with "Unable to locate JNA native support library"
    // (hit exactly this with a bundled 5.14.0 against a platform bundling
    // 7.0.4). This also means we track whatever JNA version each IDE
    // build ships, automatically, instead of one pinned number that only
    // works against a specific IDE build.
    compileOnly("net.java.dev.jna:jna:5.14.0")
}

intellijPlatform {
    pluginConfiguration {
        id = "dev.guicons.idea-plugin"
        name = "guicons"
        version = project.version.toString()
        description = """
            Native icon preview for guicons' icon!/icon_key!/icon_data! macro
            calls - renders the resolved SVG/PNG directly in the Quick
            Documentation popup, instead of only a text description.
        """.trimIndent()

        ideaVersion {
            sinceBuild = "252"
        }
    }
}

kotlin {
    // 21, not the host JDK's own version - this is the bytecode/runtime
    // level the *target platform* (RustRover 2024.3's bundled JBR) needs
    // to be able to load, not a matter of what Kotlin can compile with.
    // A newer host JDK compiling *for* a newer target than the platform
    // supports fails at plugin load time with UnsupportedClassVersionError,
    // regardless of Kotlin's own version. Gradle will provision a JDK 21
    // toolchain itself if none is registered - no need to fight IntelliJ's
    // own (separate, currently a bit broken-looking) SDK picker for this.
    jvmToolchain(21)
}

// The checked-in `src/main/resources/win32-x86-64/guicons_ffi.dll` and
// `src/main/kotlin/uniffi/guicons_ffi/guicons_ffi.kt` used to be
// hand-copied after every `crates/guicons-ffi` change - and, predictably,
// went stale relative to each other (a real UniFFI checksum-mismatch
// crash at runtime was hit exactly this way). Both are now regenerated
// from the same `cargo build` output on every Gradle build instead, so
// they can never drift apart.
val repoRoot = layout.projectDirectory.dir("..").asFile
val ffiCrateDir = repoRoot.resolve("crates/guicons-ffi")
val ffiReleaseDll = repoRoot.resolve("target/release/guicons_ffi.dll")
val uniffiBindingsOutDir = layout.buildDirectory.dir("generated/uniffi")
val lspReleaseExe = repoRoot.resolve("target/release/guicons-lsp.exe")

val cargoBuildFfi = tasks.register<Exec>("cargoBuildFfi") {
    description = "Builds ../crates/guicons-ffi in release mode"
    workingDir = repoRoot
    commandLine("cargo", "build", "--release", "-p", "guicons-ffi")
    inputs.dir(repoRoot.resolve("crates/guicons-ffi/src"))
    inputs.dir(repoRoot.resolve("crates/guicons-core/src"))
    outputs.file(ffiReleaseDll)
}

val generateUniffiBindings = tasks.register<Exec>("generateUniffiBindings") {
    dependsOn(cargoBuildFfi)
    description = "Regenerates the Kotlin UniFFI bindings from the just-built native library"
    workingDir = ffiCrateDir
    commandLine(
        "cargo", "run", "--release", "--bin", "uniffi-bindgen", "--features", "uniffi/cli", "--",
        "generate", "--library", ffiReleaseDll.absolutePath,
        "--language", "kotlin", "--out-dir", uniffiBindingsOutDir.get().asFile.absolutePath,
    )
    inputs.file(ffiReleaseDll)
    outputs.dir(uniffiBindingsOutDir)
}

val syncNativeLibrary = tasks.register<Copy>("syncNativeLibrary") {
    description = "Copies the freshly built native library into the plugin's resources"
    dependsOn(cargoBuildFfi)
    from(ffiReleaseDll)
    into(layout.projectDirectory.dir("src/main/resources/win32-x86-64"))
}

val syncGeneratedBindings = tasks.register<Copy>("syncGeneratedBindings") {
    description = "Overwrites the checked-in Kotlin bindings with the freshly regenerated ones"
    dependsOn(generateUniffiBindings)
    from(uniffiBindingsOutDir.get().dir("uniffi/guicons_ffi"))
    into(layout.projectDirectory.dir("src/main/kotlin/uniffi/guicons_ffi"))
}

// Bundles the `guicons-lsp` binary the same way `guicons_ffi.dll` is
// bundled above - built once here, shipped inside the plugin's own
// resources, so users never need it on PATH (mirrors how the Prisma ORM
// IntelliJ plugin bundles its own language server rather than requiring
// a system install).
val cargoBuildLsp = tasks.register<Exec>("cargoBuildLsp") {
    description = "Builds ../crates/guicons-lsp in release mode"
    workingDir = repoRoot
    commandLine("cargo", "build", "--release", "-p", "guicons-lsp")
    inputs.dir(repoRoot.resolve("crates/guicons-lsp/src"))
    inputs.dir(repoRoot.resolve("crates/guicons-core/src"))
    outputs.file(lspReleaseExe)
}

val syncLspBinary = tasks.register<Copy>("syncLspBinary") {
    description = "Copies the freshly built guicons-lsp binary into the plugin's resources"
    dependsOn(cargoBuildLsp)
    from(lspReleaseExe)
    into(layout.projectDirectory.dir("src/main/resources/win32-x86-64"))
}

tasks.named("processResources") { dependsOn(syncNativeLibrary, syncLspBinary) }
tasks.named("compileKotlin") { dependsOn(syncGeneratedBindings) }

tasks {
    wrapper {
        gradleVersion = "9.0"
    }

    // Internal mode gates a bunch of platform-only debugging UI - notably
    // Help > Diagnostic Tools > UI Inspector, used to identify exactly
    // which Swing component/color is responsible for a background
    // mismatch instead of guessing blindly. Off by default in a normal
    // IDE install; this makes the sandbox launched by `runIde` always
    // have it on, no manual "Internal Mode" action needed each run.
    named<JavaExec>("runIde") {
        systemProperty("idea.is.internal", "true")
    }
}
