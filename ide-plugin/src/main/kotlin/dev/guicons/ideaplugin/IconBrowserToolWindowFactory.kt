package dev.guicons.ideaplugin

import com.intellij.openapi.fileEditor.FileEditorManager
import com.intellij.openapi.fileEditor.FileEditorManagerEvent
import com.intellij.openapi.fileEditor.FileEditorManagerListener
import com.intellij.openapi.project.Project
import com.intellij.openapi.wm.ToolWindow
import com.intellij.openapi.wm.ToolWindowFactory
import com.intellij.ui.content.ContentFactory
import java.awt.BorderLayout
import javax.swing.JLabel
import javax.swing.JPanel
import javax.swing.SwingConstants

/**
 * Registered statically in `plugin.xml` (`<toolWindow id="Guicons Icons"
 * .../>`) so the icon browser has a permanent spot in the sidebar's tool
 * window bar - it doesn't only appear after pinning a popup
 * ([IconBrowserPopup.pinToToolWindow] just reuses this same registered
 * window's content instead of registering/unregistering a throwaway one).
 *
 * Tracks the currently active `.rs` editor and rebuilds its tabs to match
 * whenever the selection changes, so opening the tool window (rather than
 * pinning a popup from a specific file) still shows something useful.
 */
class IconBrowserToolWindowFactory : ToolWindowFactory {
    override fun createToolWindowContent(project: Project, toolWindow: ToolWindow) {
        val content = ContentFactory.getInstance().createContent(placeholder(), null, false)
        toolWindow.contentManager.addContent(content)
        showForCurrentFile(project, toolWindow)

        project.messageBus.connect(toolWindow.disposable).subscribe(
            FileEditorManagerListener.FILE_EDITOR_MANAGER,
            object : FileEditorManagerListener {
                override fun selectionChanged(event: FileEditorManagerEvent) = showForCurrentFile(project, toolWindow)
            },
        )
    }

    private fun showForCurrentFile(project: Project, toolWindow: ToolWindow) {
        val content = toolWindow.contentManager.contents.firstOrNull() ?: return
        val editor = FileEditorManager.getInstance(project).selectedTextEditor
        content.component = if (editor != null && editor.virtualFile?.extension == "rs") {
            buildIconBrowserTabs(project, editor, editor.virtualFile!!.path, vertical = true)
        } else {
            placeholder()
        }
    }

    private fun placeholder(): JPanel =
        JPanel(BorderLayout()).apply { add(JLabel("Open a .rs file to browse icons", SwingConstants.CENTER), BorderLayout.CENTER) }
}
