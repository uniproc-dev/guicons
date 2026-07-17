package dev.guicons.ideaplugin

import com.intellij.execution.configurations.GeneralCommandLine
import com.intellij.openapi.diagnostic.logger
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.platform.lsp.api.LspServerDescriptor
import com.intellij.platform.lsp.api.LspServerSupportProvider
import com.intellij.platform.lsp.api.ProjectWideLspServerDescriptor
import java.io.File
import java.security.MessageDigest

/**
 * Hooks `guicons-lsp` into RustRover through the platform's native LSP
 * client (`com.intellij.platform.lsp.api`), current stable names as of
 * RustRover 2025.2 - JetBrains has announced a future rename
 * (`LspServerSupportProvider` -> `LspIntegrationProvider`, `LspServer` ->
 * `LspClient`) but explicitly advises against migrating ahead of need, and
 * the SDK docs still document these names as of this writing.
 *
 * `guicons-lsp` already covers real ground the rest of this plugin
 * doesn't: hover/goto-definition/diagnostics for `icons.gui.toml` itself
 * (nothing else in this plugin looks at that file's semantics at all), and
 * goto-definition for `.rs` macro call sites (jumping from `icon!(...)` to
 * the manifest entry it resolves to). `.rs` hover is intentionally left to
 * the platform's own LSP hover tooltip here too, on top of this plugin's
 * existing custom Quick Doc ([GuiconsQuickDocAction]) - a minor surface
 * overlap (mouse-hover tooltip vs. Ctrl+Q popup) rather than a conflict,
 * since Quick Doc still wins for the literal-argument case the platform
 * providers can't resolve on their own.
 */
class GuiconsLspServerSupportProvider : LspServerSupportProvider {
    override fun fileOpened(project: Project, file: VirtualFile, serverStarter: LspServerSupportProvider.LspServerStarter) {
        if (!isGuiconsFile(file)) return
        val binary = GuiconsLspBinary.resolve() ?: return
        serverStarter.ensureServerStarted(GuiconsLspServerDescriptor(project, binary))
    }
}

private fun isGuiconsFile(file: VirtualFile) = file.extension == "rs" || file.name.endsWith(".gui.toml")

private class GuiconsLspServerDescriptor(project: Project, private val binary: File) :
    ProjectWideLspServerDescriptor(project, "Guicons") {
    override fun isSupportedFile(file: VirtualFile) = isGuiconsFile(file)
    override fun createCommandLine() = GeneralCommandLine(binary.absolutePath)
}

/**
 * `guicons-lsp.exe` ships inside the plugin's own jar (built and copied in
 * alongside `guicons_ffi.dll` by `ide-plugin/build.gradle.kts`'s
 * `syncLspBinary` task) rather than requiring it on `PATH`, mirroring how
 * the Prisma ORM IntelliJ plugin bundles its own language server. Unlike
 * the FFI `.dll` (loaded straight off the classpath via JNA), an LSP
 * server has to be a real file on disk to hand to [GeneralCommandLine] as
 * a child process - extracted once into the IDE's own per-plugin cache
 * dir, keyed by content hash so a plugin upgrade doesn't keep running a
 * stale extracted copy.
 */
private object GuiconsLspBinary {
    private val LOG = logger<GuiconsLspBinary>()

    fun resolve(): File? {
        val resourceStream = GuiconsLspBinary::class.java.getResourceAsStream("/win32-x86-64/guicons-lsp.exe")
        if (resourceStream == null) {
            LOG.warn("guicons-lsp.exe not found on the plugin classpath - LSP features disabled")
            return null
        }
        val bytes = resourceStream.use { it.readBytes() }
        val digest = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }.take(16)

        val cacheDir = File(System.getProperty("java.io.tmpdir"), "guicons-lsp-cache")
        cacheDir.mkdirs()
        val target = File(cacheDir, "guicons-lsp-$digest.exe")
        if (!target.exists()) {
            target.writeBytes(bytes)
        }
        return target
    }
}
