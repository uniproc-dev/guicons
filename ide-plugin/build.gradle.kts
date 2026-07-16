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
    implementation("org.apache.xmlgraphics:batik-transcoder:1.17")
    implementation("org.apache.xmlgraphics:batik-codec:1.17")

    // Loads ../crates/guicons-ffi's native library - the generated UniFFI
    // Kotlin bindings (src/main/kotlin/uniffi/guicons_ffi/guicons_ffi.kt)
    // use JNA's `Native.load` to call into it. Version pinned to match
    // whatever the bindings were generated against; bump both together.
    implementation("net.java.dev.jna:jna:5.14.0")
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

tasks {
    wrapper {
        gradleVersion = "9.0"
    }
}
