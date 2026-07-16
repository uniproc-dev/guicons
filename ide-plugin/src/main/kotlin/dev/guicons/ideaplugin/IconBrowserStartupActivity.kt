package dev.guicons.ideaplugin

import com.intellij.openapi.project.Project
import com.intellij.openapi.startup.ProjectActivity
import com.intellij.openapi.wm.ToolWindowManager

/**
 * Shows the "Guicons Icons" tool window ([IconBrowserToolWindowFactory])
 * as soon as a project opens, so it reads as a permanent, always-docked
 * part of the sidebar rather than something the user has to go find and
 * click open in the stripe first.
 *
 * `ToolWindowManager.invokeLater` (not a plain `Application.invokeLater`/
 * `Dispatchers.EDT` hop) is the platform's own documented way to schedule
 * tool-window operations - besides landing on the EDT the way any
 * `ToolWindow.show()` call requires, it's specifically sequenced with the
 * manager's own pending work, which a raw EDT hop isn't. That's what
 * covers the case this activity actually hit: `getToolWindow` returning
 * `null` because the `<toolWindow>` extension's registration hadn't
 * finished yet when this ran.
 */
class IconBrowserStartupActivity : ProjectActivity {
    override suspend fun execute(project: Project) {
        val manager = ToolWindowManager.getInstance(project)
        manager.invokeLater { manager.getToolWindow(IconBrowserPopup.TOOL_WINDOW_ID)?.show() }
    }
}
