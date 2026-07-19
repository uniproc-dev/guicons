package dev.guicons.ideaplugin

import com.intellij.codeInsight.documentation.DocumentationManager
import com.intellij.openapi.actionSystem.ActionUpdateThread
import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.actionSystem.CommonDataKeys
import com.intellij.openapi.ui.popup.JBPopupFactory
import com.intellij.ui.components.JBScrollPane
import com.intellij.util.ui.HTMLEditorKitBuilder
import com.intellij.util.ui.UIUtil
import uniffi.guicons_ffi.macroCallAt
import javax.swing.JEditorPane

/**
 * Overrides the platform's own `QuickJavaDoc` (Ctrl+Q/F1) action - but only
 * for the one case the normal `DocumentationTarget`/`DocumentationProvider`
 * machinery ([GuiconsDocumentationProvider],
 * [GuiconsPsiDocumentationTargetProvider]) can't reach at all: a
 * string-literal macro argument (`icon!("family/variant")`).
 * `ShowQuickDocInfoAction.update` only enables the action once it finds a
 * non-empty `DOCUMENTATION_TARGETS` data-context entry, and the Rust
 * plugin never populates one for a bare string-literal token (confirmed:
 * *no* string literal anywhere in a `.rs` file gets one, not just macro
 * arguments) - so neither provider is ever invoked for that case, no
 * matter how they're registered.
 *
 * The dotted-path form (`icon!(family.variant)`) and everything unrelated
 * to guicons already work fine through the normal providers - this class
 * deliberately leaves those to [DocumentationManager] (the legacy,
 * genuinely public entry point `GuiconsDocumentationProvider` itself
 * plugs into), only building its own popup for the string-literal case
 * the platform can't resolve on its own. Extends the plain `AnAction`,
 * not `ShowQuickDocInfoAction` - that class is `@ApiStatus.Internal`
 * (confirmed via the Marketplace's Plugin Verifier flagging it), so
 * subclassing it to reuse `update`/`actionPerformed` would itself be an
 * internal-API violation.
 */
class GuiconsQuickDocAction : AnAction() {
    override fun getActionUpdateThread(): ActionUpdateThread = ActionUpdateThread.BGT

    override fun update(e: AnActionEvent) {
        e.presentation.isEnabled = renderAt(e) != null || defaultTargetElement(e) != null
    }

    override fun actionPerformed(e: AnActionEvent) {
        val html = renderAt(e)
        if (html != null) {
            showPopup(e, html)
            return
        }
        val project = e.project ?: return
        val editor = e.getData(CommonDataKeys.EDITOR) ?: return
        val file = e.getData(CommonDataKeys.PSI_FILE) ?: return
        DocumentationManager.getInstance(project).showJavaDocInfo(editor, file, true)
    }

    private fun defaultTargetElement(e: AnActionEvent) = e.project?.let { project ->
        val editor = e.getData(CommonDataKeys.EDITOR) ?: return@let null
        val file = e.getData(CommonDataKeys.PSI_FILE) ?: return@let null
        DocumentationManager.getInstance(project).findTargetElement(editor, file)
    }

    /** Only the string-literal case - the dotted-path form is left to `super`. */
    private fun renderAt(e: AnActionEvent): String? {
        val editor = e.getData(CommonDataKeys.EDITOR) ?: return null
        val file = e.getData(CommonDataKeys.PSI_FILE) ?: return null
        val virtualFile = file.virtualFile ?: return null
        if (virtualFile.extension != "rs") return null

        val offset = editor.caretModel.offset
        val site = macroCallAt(file.text, offset.toUInt()) ?: return null
        if (!site.argText.trimStart().startsWith("\"")) return null

        return IconDocRenderer.render(virtualFile, file.text, offset)
    }

    private fun showPopup(e: AnActionEvent, html: String) {
        val editor = e.getData(CommonDataKeys.EDITOR) ?: return
        // Plain `JEditorPane("text/html", ...)` uses Swing's own default
        // HTML kit - system font, no IDE theming, looks straight out of
        // Java 1.4. `HTMLEditorKitBuilder` is what the platform's own
        // popups (including the normal Quick Doc one) use to actually
        // look like part of the IDE.
        val pane = JEditorPane().apply {
            editorKit = HTMLEditorKitBuilder().withWordWrapViewFactory().build()
            isEditable = false
            isOpaque = false
            font = UIUtil.getLabelFont()
            text = "<html><body>$html</body></html>"
        }
        JBPopupFactory.getInstance()
            .createComponentPopupBuilder(JBScrollPane(pane), pane)
            .setResizable(true)
            .setMovable(true)
            // `setRequestFocus(true)` is what made a single click on the
            // editor afterward only dismiss the popup instead of also
            // landing the click in the editor - the platform's own Quick
            // Doc popup doesn't grab focus this way, so a click both
            // closes it and acts on the editor in one go.
            .setRequestFocus(false)
            .setFocusable(true)
            .setCancelOnClickOutside(true)
            .setCancelOnOtherWindowOpen(true)
            .setCancelKeyEnabled(true)
            .createPopup()
            .showInBestPositionFor(editor)
    }
}
