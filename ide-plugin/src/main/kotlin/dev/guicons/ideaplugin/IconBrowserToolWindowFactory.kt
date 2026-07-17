package dev.guicons.ideaplugin

import com.intellij.openapi.fileEditor.FileEditorManager
import com.intellij.openapi.fileEditor.FileEditorManagerEvent
import com.intellij.openapi.fileEditor.FileEditorManagerListener
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFileManager
import com.intellij.openapi.vfs.newvfs.BulkFileListener
import com.intellij.openapi.vfs.newvfs.events.VFileEvent
import com.intellij.openapi.wm.ToolWindow
import com.intellij.openapi.wm.ToolWindowFactory
import com.intellij.ui.content.ContentFactory
import kotlinx.coroutines.launch
import java.awt.BorderLayout
import java.io.File
import javax.swing.JLabel
import javax.swing.JPanel
import javax.swing.SwingConstants
import uniffi.guicons_ffi.ListManifestEntriesOutcome
import uniffi.guicons_ffi.listManifestEntries
import uniffi.guicons_ffi.listWorkspaceManifests

/**
 * Registered statically in `plugin.xml` (`<toolWindow id="Guicons" .../>`)
 * so the icon browser has a permanent spot in the sidebar's tool
 * window bar - it doesn't only appear after pinning a popup
 * ([IconBrowserPopup.pinToToolWindow] just reuses this same registered
 * window's content instead of registering/unregistering a throwaway one).
 *
 * Tracks the active editor and rebuilds its tabs to match whenever the
 * selection changes, so opening the tool window (rather than pinning a
 * popup from a specific file) still shows something useful. The
 * workspace-wide manifest index backing that ([ManifestIndex]) only gets
 * rebuilt when an `icons.gui.toml` actually changes on disk, not on every
 * selection change - see the `VFS_CHANGES` subscription below.
 */
class IconBrowserToolWindowFactory : ToolWindowFactory {
    /** Whether real tabs have ever been shown - once true, an unrelated
     * file selection just leaves them up instead of reverting to the
     * placeholder (see [showForCurrentFile]'s doc comment). */
    private var hasShownTabs = false

    override fun createToolWindowContent(project: Project, toolWindow: ToolWindow) {
        val scope = IconBrowserScope.of(project)
        val manifestIndex = ManifestIndex(project)
        val content = ContentFactory.getInstance().createContent(placeholder(), null, false)
        toolWindow.contentManager.addContent(content)

        scope.launch {
            manifestIndex.refresh()
            showForCurrentFile(project, toolWindow, manifestIndex)
        }

        project.messageBus.connect(toolWindow.disposable).subscribe(
            FileEditorManagerListener.FILE_EDITOR_MANAGER,
            object : FileEditorManagerListener {
                override fun selectionChanged(event: FileEditorManagerEvent) = showForCurrentFile(project, toolWindow, manifestIndex)
            },
        )

        // The index only actually goes stale when a manifest itself is
        // added/removed/edited - not on every editor selection change,
        // which is unrelated to whether any manifest changed. `[link]
        // includes = [...]` is just a list of path strings
        // (`graph.rs::extract_includes`) - nothing requires an included
        // file to be named `*.gui.toml` itself, so that suffix alone would
        // miss an edit to an already-linked file with an arbitrary name.
        // Covered two ways instead: the suffix catches a *new* root/linked
        // manifest showing up (nothing in the index yet to match against),
        // and `manifestIndex.manifestFor` catches an edit to a file
        // that's *already* indexed regardless of what it's named -
        // between the two, only a manifest neither seen before nor
        // `.gui.toml`-suffixed could slip through, and that file wouldn't
        // be discoverable as a root manifest by `find_manifest_files`
        // either way. Rebuilding wholesale (not patching the one changed
        // manifest's slice of the index) rather than tracking
        // incrementally, matching `ManifestIndex.refresh`'s own tradeoff.
        project.messageBus.connect(toolWindow.disposable).subscribe(
            VirtualFileManager.VFS_CHANGES,
            object : BulkFileListener {
                override fun after(events: MutableList<out VFileEvent>) {
                    val relevant = events.any { event ->
                        val file = event.file ?: return@any false
                        file.name.endsWith(".gui.toml") || manifestIndex.manifestFor(file.path) != null
                    }
                    if (!relevant) return
                    scope.launch {
                        manifestIndex.refresh()
                        showForCurrentFile(project, toolWindow, manifestIndex)
                    }
                }
            },
        )
    }

    /** An `.rs` file needs its own text editor to insert into and drives
     * the tabs' content. A non-`.rs` file - a manifest itself, or one of
     * its declared assets, anywhere in the workspace - doesn't get its own
     * tabs, but shouldn't blank the browser out either: [ManifestIndex]
     * decides that by looking the file up in a reverse index built from
     * *every* manifest under the workspace root (an in-memory cache -
     * [ManifestIndex.manifestFor] never touches disk itself, only the
     * background [ManifestIndex.refresh] does), not just "whichever
     * manifest was last shown" - a `[link] includes` entry or an asset's
     * `source` can point anywhere on disk, and the workspace can have more
     * than one crate/manifest in the first place.
     *
     * A file that's neither doesn't get cleared either, once *something*
     * real has been shown at least once - switching to a scratch file, a
     * terminal tab, whatever, shouldn't blank out a browser the user was
     * actively using a moment ago. Only the very first selection, before
     * anything relevant has ever been found, falls through to the
     * placeholder. */
    private fun showForCurrentFile(project: Project, toolWindow: ToolWindow, manifestIndex: ManifestIndex) {
        val content = toolWindow.contentManager.contents.firstOrNull() ?: return
        val editor = FileEditorManager.getInstance(project).selectedTextEditor
        val file = FileEditorManager.getInstance(project).selectedFiles.firstOrNull()
        when {
            editor != null && file != null && file.extension == "rs" -> {
                content.component = buildIconBrowserTabs(project, editor, file.path, vertical = true)
                hasShownTabs = true
            }
            file != null && manifestIndex.manifestFor(file.path) != null -> Unit
            !hasShownTabs -> content.component = placeholder()
            else -> Unit
        }
    }

    private fun placeholder(): JPanel =
        JPanel(BorderLayout()).apply { add(JLabel("Open a file inside a guicons-managed crate to browse icons", SwingConstants.CENTER), BorderLayout.CENTER) }
}

/** Reverse index - "which manifest, if any, owns this file" - built by
 * scanning the whole workspace for `icons.gui.toml` files
 * ([listWorkspaceManifests]) and, for each one, resolving every entry it
 * declares ([listManifestEntries]) into the manifest file itself, each
 * entry's asset (`sourceFile`), and each entry's declaring file
 * (`declaredInFilePath`, distinct from the root manifest for a `[link]`d
 * one). Built once and rebuilt wholesale rather than tracked
 * incrementally - a directory walk plus parsing a handful of manifests is
 * cheap enough not to bother, same tradeoff `guicons-lsp`'s own workspace
 * scan already makes. */
private class ManifestIndex(private val project: Project) {
    @Volatile private var fileToManifest: Map<String, String> = emptyMap()

    suspend fun refresh() {
        val root = project.basePath ?: return
        val index = HashMap<String, String>()
        for (manifestPath in listWorkspaceManifests(root)) {
            index[canonical(manifestPath)] = manifestPath
            val entries = when (val outcome = listManifestEntries(manifestPath)) {
                is ListManifestEntriesOutcome.Found -> outcome.entries
                is ListManifestEntriesOutcome.ManifestInvalid -> continue
            }
            entries.forEach { entry ->
                entry.sourceFile?.let { index[canonical(it)] = manifestPath }
                index[canonical(entry.declaredInFilePath)] = manifestPath
            }
        }
        fileToManifest = index
    }

    fun manifestFor(filePath: String): String? = fileToManifest[canonical(filePath)]

    /// Not a raw string comparison - handles Windows' `\\?\` verbatim-path
    /// prefix (which `guicons-core`'s path resolution keeps but a
    /// `VirtualFile.path` never has) and any `.`/`..` difference between
    /// the two sides without needing to know which of them might have
    /// either.
    private fun canonical(path: String): String = runCatching { File(path).canonicalFile.path }.getOrDefault(path)
}
