package dev.guicons.ideaplugin

import com.intellij.lang.documentation.AbstractDocumentationProvider
import com.intellij.psi.PsiElement

/**
 * Quick Documentation (Ctrl+Q / F1) provider for `icon!`/`icon_key!`/
 * `icon_data!` call sites whose argument is a *referenceable* token (a
 * bare `family.variant` path) - the platform only ever offers those to
 * the legacy per-language `documentationProvider` list this class is
 * registered under. A plain string-literal argument
 * (`icon!("family/variant")`) never reaches this class at all, no matter
 * the registration order - see [GuiconsPsiDocumentationTargetProvider] for
 * that case, and [IconDocRenderer] for the rendering logic shared by both.
 */
class GuiconsDocumentationProvider : AbstractDocumentationProvider() {

    override fun generateDoc(element: PsiElement?, originalElement: PsiElement?): String? {
        val target = originalElement ?: return null
        val psiFile = target.containingFile ?: return null
        val virtualFile = psiFile.virtualFile ?: return null
        if (virtualFile.extension != "rs") return null

        return IconDocRenderer.render(virtualFile, psiFile.text, target.textRange.startOffset)
    }
}
