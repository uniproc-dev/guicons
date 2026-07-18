// Lets Gradle actually download the JDK 21 toolchain `build.gradle.kts`'s
// `jvmToolchain(21)` asks for (21 is what RustRover 2025.2.3, the target
// IDE, bundles as its own JBR) - without a resolver plugin registered,
// Gradle has no way to fetch a JDK it doesn't already have installed and
// silently falls back to whatever JAVA_HOME happens to be, which breaks
// the moment that's a JDK newer than this Kotlin compiler version knows
// about.
plugins {
    id("org.gradle.toolchains.foojay-resolver-convention") version "0.9.0"
}

rootProject.name = "guicons-idea-plugin"
