package dev.guicons.ideaplugin

import com.intellij.model.Pointer
import com.intellij.openapi.diagnostic.thisLogger
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.platform.backend.documentation.DocumentationResult
import com.intellij.platform.backend.documentation.DocumentationTarget
import com.intellij.platform.backend.documentation.PsiDocumentationTargetProvider
import com.intellij.platform.backend.presentation.TargetPresentation
import com.intellij.psi.PsiElement

/**
 * Lower-level counterpart to [GuiconsDocumentationProvider]: the platform
 * only ever offers a *referenceable* PSI element (an identifier-shaped
 * token) to the legacy per-language `documentationProvider` list, never a
 * bare string-literal token - `icon!("family/variant")`'s argument is
 * exactly that, so it needs this newer, non-language-scoped
 * `PsiDocumentationTargetProvider` extension point to be reachable at all
 * (confirmed by instrumenting both: the legacy provider was invoked for
 * every identifier hovered, but never once for a string literal, with or
 * without `order="first"`).
 */
class GuiconsPsiDocumentationTargetProvider : PsiDocumentationTargetProvider {
    override fun documentationTarget(element: PsiElement, originalElement: PsiElement?): DocumentationTarget? {
        val target = originalElement ?: element
        val psiFile = target.containingFile ?: return null
        val virtualFile = psiFile.virtualFile ?: return null
        if (virtualFile.extension != "rs") return null

        val offset = target.textRange.startOffset
        // TEMPORARY diagnostic logging - remove once the string-literal
        // selector case (icon!("family/variant")) is confirmed working.
        thisLogger().warn(
            "guicons psiDocTarget: element=${element.javaClass.name} originalElement=${originalElement?.javaClass?.name} " +
                "offset=$offset text=${target.text}"
        )

        val html = IconDocRenderer.render(virtualFile, psiFile.text, offset) ?: return null
        return GuiconsDocumentationTarget(html)
    }
}

private class GuiconsDocumentationTarget(private val html: String) : DocumentationTarget {
    override fun createPointer(): Pointer<out DocumentationTarget> = Pointer.hardPointer(this)

    override fun computePresentation(): TargetPresentation = TargetPresentation.builder("guicons icon").presentation()

    override fun computeDocumentation(): DocumentationResult = DocumentationResult.documentation(html)
}
