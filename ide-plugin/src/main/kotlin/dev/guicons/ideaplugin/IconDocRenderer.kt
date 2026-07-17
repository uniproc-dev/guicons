package dev.guicons.ideaplugin

import com.intellij.openapi.vfs.VirtualFile
import uniffi.guicons_ffi.IconSelector
import uniffi.guicons_ffi.ResolveOutcome
import uniffi.guicons_ffi.findManifestForRustFile
import uniffi.guicons_ffi.macroCallAt
import uniffi.guicons_ffi.parseSelector
import uniffi.guicons_ffi.resolveFamilyVariant
import java.io.ByteArrayInputStream
import java.io.File
import java.util.Base64
import javax.imageio.ImageIO

/**
 * Shared HTML-popup rendering for an `icon!`/`icon_key!`/`icon_data!` call
 * site - used by both [GuiconsDocumentationProvider] (the legacy
 * `generateDoc` API, which the platform only invokes for elements it
 * considers "referenceable", e.g. a bare `family.variant` path) and
 * [GuiconsPsiDocumentationTargetProvider] (the newer
 * `PsiDocumentationTargetProvider` API, needed for a plain string-literal
 * argument like `icon!("family/variant")` - the platform never offers a
 * bare literal token to the legacy per-language `documentationProvider`
 * list at all, regardless of registration order, so that case needs this
 * separate, lower-level extension point).
 */
object IconDocRenderer {
    /** `null` if `offset` in `text` isn't inside a guicons macro call. */
    fun render(virtualFile: VirtualFile, text: String, offset: Int): String? {
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

    /** Renders onto an [IconPreviewCard] (same contrast-with-the-icon's-own-
     * pixels logic as the icon browser popup's card) rather than embedding
     * the raw icon bytes directly - an SVG with no fill set renders solid
     * black, otherwise invisible against a doc popup that's dark-themed
     * itself. */
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
        val image = ImageIO.read(ByteArrayInputStream(pngBytes)) ?: return null
        val cardBytes = IconPreviewCard.renderCardPng(image, cardSize = CARD_SIZE, arc = CARD_ARC)
        val base64 = Base64.getEncoder().encodeToString(cardBytes)
        return "<img src=\"data:image/png;base64,$base64\" width=\"$CARD_SIZE\" height=\"$CARD_SIZE\"/>"
    }

    private const val CARD_SIZE = 80
    private const val CARD_ARC = 16
}
