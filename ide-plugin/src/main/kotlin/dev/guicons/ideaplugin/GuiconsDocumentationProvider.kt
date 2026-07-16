package dev.guicons.ideaplugin

import com.intellij.lang.documentation.AbstractDocumentationProvider
import com.intellij.psi.PsiElement
import uniffi.guicons_ffi.IconSelector
import uniffi.guicons_ffi.ResolveOutcome
import uniffi.guicons_ffi.findManifestForRustFile
import uniffi.guicons_ffi.macroCallAt
import uniffi.guicons_ffi.parseSelector
import uniffi.guicons_ffi.resolveFamilyVariant
import java.io.File
import java.util.Base64

/**
 * Quick Documentation (Ctrl+Q / F1) provider for `icon!`/`icon_key!`/
 * `icon_data!` call sites - the native-rendering counterpart to
 * guicons-lsp's own text-only hover. Registered for the Rust language in
 * `plugin.xml`.
 *
 * All detection/parsing/manifest-resolution logic lives in
 * `../../crates/guicons-ffi` (a thin UniFFI wrapper around
 * `guicons-core`, the exact same already-tested Rust code
 * `guicons-lsp` uses) - this class only calls into the generated
 * bindings (`src/main/kotlin/uniffi/guicons_ffi/guicons_ffi.kt`, checked
 * in rather than generated at build time for now - see the repo README
 * for how to regenerate) and handles what's genuinely native-UI-specific:
 * rasterizing the resolved asset and building the HTML popup.
 */
class GuiconsDocumentationProvider : AbstractDocumentationProvider() {

    override fun generateDoc(element: PsiElement?, originalElement: PsiElement?): String? {
        val target = originalElement ?: return null
        val psiFile = target.containingFile ?: return null
        val virtualFile = psiFile.virtualFile ?: return null
        if (virtualFile.extension != "rs") return null

        val text = psiFile.text
        val offset = target.textRange.startOffset
        val site = macroCallAt(text, offset.toUInt()) ?: return null
        val selector = parseSelector(site.argText) ?: return null

        return when (selector) {
            is IconSelector.Iconify -> """
                <b>${selector.id}</b><br/>
                raw iconify id - resolved directly through <code>guicons-net</code>'s
                cache, no manifest entry for this one
            """.trimIndent()

            is IconSelector.FamilyVariant -> renderFamilyVariant(File(virtualFile.path), selector)
        }
    }

    private fun renderFamilyVariant(rustFile: File, selector: IconSelector.FamilyVariant): String {
        val key = listOfNotNull(selector.family, selector.size?.toString(), selector.variant).joinToString("-")
        val manifestPath = findManifestForRustFile(rustFile.path)
            ?: return "<b>$key</b><br/>no icons.gui.toml found for this crate"

        return when (val outcome = resolveFamilyVariant(manifestPath, selector.family, selector.size, selector.variant)) {
            is ResolveOutcome.NotFound -> "<b>$key</b><br/>not found in ${File(manifestPath).name}"

            is ResolveOutcome.ManifestInvalid -> buildString {
                append("<b>").append(key).append("</b><br/>")
                append(File(manifestPath).name).append(" failed to load:<br/>")
                outcome.errors.forEach { append(it).append("<br/>") }
            }

            is ResolveOutcome.Found -> buildString {
                val entry = outcome.v1
                append("<b>").append(entry.key).append("</b><br/>")
                append("source: ").append(entry.sourceDescription).append("<br/>")
                val preview = entry.sourceFile?.let { renderPreview(File(it)) }
                append(preview ?: "(preview unavailable - iconify/url/glyph sources aren't rendered yet)")
            }
        }
    }

    private fun renderPreview(asset: File): String? {
        if (!asset.isFile) return null
        val pngBytes = try {
            when (asset.extension.lowercase()) {
                "png" -> asset.readBytes()
                "svg" -> SvgRenderer.renderToPngBytes(asset, 64)
                else -> return null // e.g. .ico - not previewed yet
            }
        } catch (_: Exception) {
            return null
        }
        val base64 = Base64.getEncoder().encodeToString(pngBytes)
        return "<img src=\"data:image/png;base64,$base64\" width=\"64\" height=\"64\"/>"
    }
}
