package dev.guicons.ideaplugin

import com.intellij.openapi.actionSystem.ActionUpdateThread
import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.actionSystem.CommonDataKeys

/**
 * Editor right-click menu entry ("Browse Icons...") - opens
 * [IconBrowserPopup], the opposite direction from Quick Doc: instead of
 * showing what an already-written `icon!(...)` call resolves to, lets the
 * user find an icon (from the current crate's manifest, or from
 * iconify.design) and insert a reference to it.
 */
class OpenIconBrowserAction : AnAction() {
    override fun getActionUpdateThread(): ActionUpdateThread = ActionUpdateThread.BGT

    override fun update(e: AnActionEvent) {
        e.presentation.isEnabledAndVisible = e.getData(CommonDataKeys.PSI_FILE)?.virtualFile?.extension == "rs"
    }

    override fun actionPerformed(e: AnActionEvent) {
        val project = e.project ?: return
        val editor = e.getData(CommonDataKeys.EDITOR) ?: return
        val file = e.getData(CommonDataKeys.PSI_FILE) ?: return
        IconBrowserPopup(project, editor, file.virtualFile.path).show()
    }
}
