package dev.guicons.ideaplugin

import com.intellij.openapi.application.EDT
import com.intellij.openapi.editor.Editor
import com.intellij.openapi.fileEditor.FileEditorManager
import com.intellij.openapi.project.Project
import com.intellij.openapi.ui.popup.JBPopupFactory
import com.intellij.openapi.vfs.LocalFileSystem
import com.intellij.openapi.wm.ToolWindowManager
import com.intellij.icons.AllIcons
import com.intellij.ui.ColoredTreeCellRenderer
import com.intellij.ui.JBColor
import com.intellij.ui.JBSplitter
import com.intellij.ui.SearchTextField
import com.intellij.ui.components.JBScrollPane
import com.intellij.ui.components.JBTabbedPane
import com.intellij.ui.content.ContentFactory
import com.intellij.ui.treeStructure.Tree
import com.intellij.util.ui.JBFont
import com.intellij.util.ui.JBUI
import com.intellij.util.ui.UIUtil
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.guicons_ffi.ListManifestEntriesOutcome
import uniffi.guicons_ffi.ResolvedEntry
import uniffi.guicons_ffi.cachedIconifyCollectionNames
import uniffi.guicons_ffi.ensureIconifyIconCached
import uniffi.guicons_ffi.findManifestForRustFile
import uniffi.guicons_ffi.listManifestEntries
import uniffi.guicons_ffi.searchIconifyIcons
import java.awt.BorderLayout
import java.awt.Color
import java.awt.Cursor
import java.awt.Dimension
import java.awt.FlowLayout
import java.awt.Graphics
import java.awt.Graphics2D
import java.awt.RenderingHints
import java.awt.event.MouseAdapter
import java.awt.event.MouseEvent
import java.awt.image.BufferedImage
import java.io.ByteArrayInputStream
import java.io.File
import javax.imageio.ImageIO
import javax.swing.BoxLayout
import javax.swing.JButton
import javax.swing.JComponent
import javax.swing.JLabel
import javax.swing.JPanel
import javax.swing.JTree
import javax.swing.SwingConstants
import javax.swing.event.DocumentEvent
import javax.swing.event.DocumentListener
import javax.swing.event.TreeSelectionListener
import javax.swing.tree.DefaultMutableTreeNode
import javax.swing.tree.DefaultTreeModel
import javax.swing.tree.TreeSelectionModel

/**
 * The opposite direction from Quick Doc: instead of showing what an
 * already-written `icon!(...)` call resolves to, lets the user *find* an
 * icon (from the current crate's manifest, or from iconify.design) and
 * insert a reference to it at the caret. Opened via the editor context
 * menu ([OpenIconBrowserAction]).
 *
 * Two tabs sharing one popup rather than two separate popups, since
 * they're really the same task ("pick an icon") with two different
 * sources.
 */
class IconBrowserPopup(private val project: Project, private val editor: Editor, private val rustFilePath: String) {
    fun show() {
        val tabs = buildTabs(vertical = false)
        tabs.preferredSize = Dimension(560, 380)

        val popup = JBPopupFactory.getInstance()
            .createComponentPopupBuilder(tabs, tabs)
            .setResizable(true)
            .setMovable(true)
            .setRequestFocus(true)
            .setFocusable(true)
            .setCancelOnClickOutside(true)
            .setCouldPin { pinToToolWindow(); true }
            .createPopup()

        popup.showInBestPositionFor(editor)
    }

    /** A floating popup is wide and short - tree left, preview right
     * makes sense there. The sidebar tool window is the opposite (narrow
     * and tall), so pinning builds a fresh set of tabs with a vertical
     * split instead (preview on top, tree below) - not just the same
     * components with the splitter's orientation flipped, since the
     * width/height a `JBSplitter` is built for isn't something you
     * re-decide after the fact without rebuilding its children's layout.
     *
     * The tool window itself is registered statically (`plugin.xml`'s
     * `<toolWindow>`, see [IconBrowserToolWindowFactory]) so it's a
     * permanent icon in the sidebar, not something that only exists after
     * a pin - this just replaces its content with tabs bound to *this*
     * popup's editor/file and shows it, overriding whatever the factory's
     * own "currently active .rs file" content had before. */
    private fun pinToToolWindow() {
        val toolWindow = ToolWindowManager.getInstance(project).getToolWindow(TOOL_WINDOW_ID) ?: return
        val content = toolWindow.contentManager.contents.firstOrNull()
        val sidebarTabs = buildTabs(vertical = true)
        if (content != null) {
            content.component = sidebarTabs
        } else {
            toolWindow.contentManager.addContent(ContentFactory.getInstance().createContent(sidebarTabs, null, false))
        }
        toolWindow.show()
    }

    private fun buildTabs(vertical: Boolean): JBTabbedPane = buildIconBrowserTabs(project, editor, rustFilePath, vertical)

    companion object {
        const val TOOL_WINDOW_ID = "Guicons Icons"
    }
}

/** Shared between [IconBrowserPopup] (both the floating popup and its
 * pin-to-sidebar path) and [IconBrowserToolWindowFactory] (the tool
 * window's own "currently active .rs file" content) - same two tabs
 * either way, just built for whichever editor/file is relevant. */
fun buildIconBrowserTabs(project: Project, editor: Editor, rustFilePath: String, vertical: Boolean): JBTabbedPane {
    val tabs = JBTabbedPane()
    tabs.background = UIUtil.getTreeBackground()
    val manifestTab = ManifestTab(project, editor, rustFilePath, vertical)
    val iconifyTab = IconifyTab(project, editor, rustFilePath, vertical)
    tabs.addTab("Manifest", manifestTab.component)
    tabs.addTab("Iconify", iconifyTab.component)
    return tabs
}

/** One node's payload in either tab's tree - a group (manifest file /
 * iconify provider prefix) just organizes its children; a leaf is the
 * thing selection previews and double-click inserts. Leaf labels use the
 * actual asset file name where there is one (`docker.svg`) - the file
 * name is what the user actually recognizes the icon by on disk. An entry
 * with no on-disk file (`iconify`/`url`/`glyph`-sourced) has no such name,
 * so falls back to its manifest key instead - still a meaningful name,
 * unlike a bare "(default)" placeholder. */
private sealed class TreeItem(val displayText: String) {
    class Group(displayText: String) : TreeItem(displayText)
    class ManifestLeaf(val entry: ResolvedEntry) : TreeItem(entry.sourceFile?.let { File(it).name } ?: entry.key)
    class IconifyLeaf(val id: String) : TreeItem(id.substringAfter(':'))
}

private fun node(item: TreeItem) = DefaultMutableTreeNode(item)

private fun treeItemOf(node: Any?): TreeItem? = (node as? DefaultMutableTreeNode)?.userObject as? TreeItem

/** `null` for a group node - selecting one shouldn't try to preview or
 * insert anything, only a leaf can. */
private val TreeItem?.asLeafOrNull: TreeItem?
    get() = this?.takeUnless { it is TreeItem.Group }

/** Resolves a root to pass into the `guicons-net` cache-path functions -
 * any path under the workspace works, since they walk up to find the
 * actual root themselves. Prefers the manifest's own directory when one
 * is found, since that's guaranteed to be inside the right crate. */
private fun iconifyCacheRoot(rustFilePath: String): String =
    findManifestForRustFile(rustFilePath)?.let { File(it).parent } ?: File(rustFilePath).parent

/** Walks up from `start` to find `.cache/guicons` the same way
 * `guicons-net`'s `workspace_cache_dir` does, purely to list what's
 * already cached on disk - not a network call. */
private fun findWorkspaceCacheDir(start: File): File {
    var dir: File? = start
    while (dir != null) {
        if (File(dir, "Cargo.toml").isFile) return File(dir, ".cache/guicons")
        dir = dir.parentFile
    }
    return File(start, ".cache/guicons")
}

/** Decoded pixels, not a Swing `Icon` - [IconCard] needs to sample the
 * actual pixel colors to pick a background that contrasts with them. */
private fun previewImage(assetPath: String?, sizePx: Int): BufferedImage? {
    if (assetPath == null) return null
    val file = File(assetPath)
    if (!file.isFile) return null
    return try {
        val pngBytes = when (file.extension.lowercase()) {
            "png" -> file.readBytes()
            "svg" -> SvgRenderer.renderToPngBytes(file, sizePx)
            else -> return null
        }
        ImageIO.read(ByteArrayInputStream(pngBytes))
    } catch (_: Exception) {
        null
    }
}

/** Average, alpha-weighted luminance of `image`'s opaque-ish pixels
 * (sparsely sampled - a preview icon is small, a full scan is wasted
 * work), picking light-on-dark-icon or dark-on-light-icon so the icon is
 * never the same shade as its own background - the exact failure mode a
 * fixed card color hit for an icon whose SVG has no fill set (renders
 * solid black, invisible against a dark-theme-colored card). */
private fun contrastingCardColor(image: BufferedImage?): Color {
    val fallback = JBColor(0xE8E8E8.toInt(), 0x3C3F41)
    if (image == null) return fallback

    var weightedLuminance = 0.0
    var totalWeight = 0.0
    val stepX = maxOf(1, image.width / 24)
    val stepY = maxOf(1, image.height / 24)
    var y = 0
    while (y < image.height) {
        var x = 0
        while (x < image.width) {
            val argb = image.getRGB(x, y)
            val alpha = (argb ushr 24) and 0xFF
            if (alpha > 16) {
                val r = (argb ushr 16) and 0xFF
                val g = (argb ushr 8) and 0xFF
                val b = argb and 0xFF
                weightedLuminance += (0.2126 * r + 0.7152 * g + 0.0722 * b) * alpha
                totalWeight += alpha
            }
            x += stepX
        }
        y += stepY
    }
    if (totalWeight == 0.0) return fallback

    return if (weightedLuminance / totalWeight < 128) Color(0xF0, 0xF0, 0xF0) else Color(0x2B, 0x2B, 0x2B)
}

private fun replaceModel(tree: Tree, root: DefaultMutableTreeNode) {
    tree.model = DefaultTreeModel(root)
    for (i in 0 until tree.rowCount) tree.expandRow(i)
}

/** A fixed-size rounded-rect card the icon preview sits on, colored to
 * contrast with the icon's *own* pixels ([contrastingCardColor]) rather
 * than a fixed theme color - a raw image floating directly on the panel
 * background looks like a rendering glitch rather than a preview, and a
 * fixed card color can still end up the same shade as the icon itself
 * (e.g. an SVG with no fill set, which just renders solid black). */
private class IconCard : JPanel() {
    var image: BufferedImage? = null
        set(value) {
            field = value
            cardColor = contrastingCardColor(value)
            repaint()
        }
    private var cardColor: Color = contrastingCardColor(null)

    init {
        isOpaque = false
        preferredSize = Dimension(CARD_SIZE, CARD_SIZE)
        minimumSize = preferredSize
        maximumSize = preferredSize
    }

    override fun paintComponent(g: Graphics) {
        super.paintComponent(g)
        val g2 = g.create() as Graphics2D
        try {
            g2.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON)
            g2.color = cardColor
            g2.fillRoundRect(0, 0, width, height, ARC, ARC)
            image?.let { g2.drawImage(it, (width - it.width) / 2, (height - it.height) / 2, null) }
        } finally {
            g2.dispose()
        }
    }

    companion object {
        private const val CARD_SIZE = 120
        private const val ARC = 16
    }
}

/// `guicons-core`'s `canonicalize_or_self` (behind every raw path this
/// popup gets from the FFI side - `source_file`/`declared_in_file_path`)
/// keeps Windows' `\\?\` verbatim-path prefix. `java.io.File` (what
/// [previewImage] reads through) tolerates it fine, but
/// `LocalFileSystem`'s own path parsing doesn't - throws
/// `InvalidPathException` on the literal `?` - so it has to come off
/// before this hits the VFS, unlike every other consumer of these paths.
private fun stripWindowsVerbatimPrefix(path: String): String = path.removePrefix("""\\?\""")

private fun openFile(project: Project, path: String) {
    val file = LocalFileSystem.getInstance().refreshAndFindFileByPath(stripWindowsVerbatimPrefix(path)) ?: return
    FileEditorManager.getInstance(project).openFile(file, true)
}

/** One `Label: value` row - clickable (opens `linkTarget` in the editor)
 * when there's actually a file to jump to; plain text otherwise (an
 * `iconify`/`url`/`glyph` source, or an `IconifyLeaf` with no manifest
 * entry at all yet, has nowhere to open). */
private fun detailRow(project: Project, label: String, value: String, linkTarget: String? = null): JComponent {
    val row = JLabel("<html><b>$label:</b> $value</html>")
    if (linkTarget != null) {
        row.cursor = Cursor.getPredefinedCursor(Cursor.HAND_CURSOR)
        row.toolTipText = linkTarget
        row.addMouseListener(object : MouseAdapter() {
            override fun mouseClicked(e: MouseEvent) = openFile(project, linkTarget)
        })
    }
    return row
}

/** `size`/`variant`/source path/declaring-file details for whatever's
 * selected - a `ManifestLeaf` has all of that from its `ResolvedEntry`
 * (with "Source"/"Declared in" clickable when there's an actual file
 * behind them); an `IconifyLeaf` is just a raw iconify id with nothing
 * else known about it yet (no manifest entry exists for it - that's the
 * whole point of browsing iconify.design for something to *add*). */
private fun detailRows(project: Project, item: TreeItem): List<JComponent> = when (item) {
    is TreeItem.Group -> emptyList()
    is TreeItem.IconifyLeaf -> listOf(detailRow(project, "Iconify id", item.id))
    is TreeItem.ManifestLeaf -> {
        val entry = item.entry
        listOfNotNull(
            detailRow(project, "Family", entry.family),
            entry.size?.let { detailRow(project, "Size", it.toString()) },
            entry.variant?.let { detailRow(project, "Variant", it) },
            detailRow(project, "Source", entry.sourceDescription, linkTarget = entry.sourceFile),
            detailRow(project, "Declared in", entry.declaredInFile, linkTarget = entry.declaredInFilePath),
        )
    }
}

/** Right-hand side of the split: a bigger preview + details for whatever
 * leaf is currently selected in the tree, plus an explicit Insert button
 * (double-click on the tree does the same thing, but a selection-only
 * click - needed anyway to *see* the preview - shouldn't also insert). */
private class PreviewPanel(private val project: Project, private val scope: CoroutineScope, private val onInsert: (TreeItem) -> Unit) {
    val component: JComponent = JPanel(BorderLayout()).apply { background = UIUtil.getTreeBackground() }
    private val iconCard = IconCard()
    private val titleLabel = JLabel("", SwingConstants.CENTER)
    private val detailsPanel = JPanel()
    private val insertButton = JButton("Insert")
    private var current: TreeItem? = null
    private var loadJob: Job? = null

    init {
        component.border = JBUI.Borders.empty(20)
        titleLabel.font = JBFont.label().asBold()
        titleLabel.border = JBUI.Borders.empty(12, 0)
        detailsPanel.layout = BoxLayout(detailsPanel, BoxLayout.Y_AXIS)
        detailsPanel.isOpaque = false
        detailsPanel.border = JBUI.Borders.emptyTop(8)
        insertButton.isEnabled = false
        insertButton.addActionListener { current?.let(onInsert) }

        val cardWrapper = JPanel()
        cardWrapper.isOpaque = false
        cardWrapper.add(iconCard)

        val center = JPanel(BorderLayout())
        center.isOpaque = false
        center.add(cardWrapper, BorderLayout.NORTH)
        center.add(titleLabel, BorderLayout.CENTER)
        center.add(detailsPanel, BorderLayout.SOUTH)

        val buttonRow = JPanel(FlowLayout(FlowLayout.CENTER, 0, 0))
        buttonRow.isOpaque = false
        buttonRow.border = JBUI.Borders.emptyTop(16)
        buttonRow.add(insertButton)

        component.add(center, BorderLayout.CENTER)
        component.add(buttonRow, BorderLayout.SOUTH)
        showNothing()
    }

    fun show(item: TreeItem?, resolveImage: suspend (TreeItem) -> BufferedImage?) {
        loadJob?.cancel()
        current = item
        if (item == null) {
            showNothing()
            return
        }
        insertButton.isEnabled = true
        iconCard.image = null
        titleLabel.text = item.displayText
        detailsPanel.removeAll()
        detailRows(project, item).forEach(detailsPanel::add)
        detailsPanel.revalidate()
        detailsPanel.repaint()
        loadJob = scope.launch {
            val image = resolveImage(item)
            withContext(Dispatchers.EDT) {
                if (current === item) iconCard.image = image
            }
        }
    }

    private fun showNothing() {
        iconCard.image = null
        titleLabel.text = "Select an icon to preview"
        detailsPanel.removeAll()
        detailsPanel.revalidate()
        detailsPanel.repaint()
        insertButton.isEnabled = false
    }
}

/** Tree and preview, separated by nothing more than `JBSplitter`'s own
 * thin divider line - horizontal (tree left, preview right) for a wide,
 * short floating popup; vertical (preview on top, tree below - the order
 * that reads naturally top-to-bottom for a narrow, tall sidebar) when
 * pinned into a tool window.
 *
 * Every prior attempt at this fought the tree itself - guessing a color
 * name to paint it with, or forcing `isOpaque` one way or the other.
 * `Help > Diagnostic Tools > UI Inspector`'d against the *platform's own*
 * project tree settled it: `ProjectViewTree` doesn't override its
 * background or opacity at all - it's a plain, untouched
 * `com.intellij.ui.treeStructure.Tree`, same base class as this one, and
 * it blends in because everything built *around* it (there, the Project
 * tool window's own panels) is colored to match the tree's own natural
 * `Tree.background`, never the other way round. So here: the tree/scroll/
 * viewport are left completely alone, and [buildIconBrowserTabs] /
 * [PreviewPanel] paint themselves with `UIUtil.getTreeBackground()`
 * instead. */
private fun buildSplit(tree: Tree, preview: JComponent, vertical: Boolean): JBSplitter {
    val splitter = JBSplitter(vertical, if (vertical) 0.45f else 0.55f)
    splitter.dividerWidth = 1
    splitter.setShowDividerControls(false)
    splitter.background = UIUtil.getTreeBackground()
    tree.border = JBUI.Borders.empty(8, 4)
    val treeScroll = JBScrollPane(tree)
    treeScroll.border = JBUI.Borders.empty()
    if (vertical) {
        splitter.firstComponent = preview
        splitter.secondComponent = treeScroll
    } else {
        splitter.firstComponent = treeScroll
        splitter.secondComponent = preview
    }
    return splitter
}

/** Selects on single click (driving the preview panel via the tree's own
 * selection listener) and inserts on double-click of a leaf row -
 * non-leaf (group) rows are left to `JTree`'s own default
 * expand/collapse handling. Uses the `editor` captured when the tab was
 * built (the popup owns the editor the whole time it's open, no need to
 * re-derive it from the clicked component). */
private fun treeDoubleClickToInsert(tree: Tree, editor: Editor, selectorFor: (TreeItem) -> String?): MouseAdapter =
    object : MouseAdapter() {
        override fun mouseClicked(e: MouseEvent) {
            if (e.clickCount != 2) return
            val path = tree.getPathForLocation(e.x, e.y) ?: return
            val item = treeItemOf(path.lastPathComponent) ?: return
            val selector = selectorFor(item) ?: return
            IconInsertion.insert(editor, selector)
        }
    }

private fun buildTree(): Tree {
    val tree = Tree(DefaultTreeModel(DefaultMutableTreeNode(TreeItem.Group("(loading...)"))))
    tree.isRootVisible = true
    tree.showsRootHandles = true
    tree.rowHeight = JBUI.scale(22)
    tree.selectionModel.selectionMode = TreeSelectionModel.SINGLE_TREE_SELECTION
    // `javax.swing.tree.DefaultTreeCellRenderer` (plain JDK Swing) was the
    // actual source of the mismatched grey box - its `paint()`
    // unconditionally `fillRect`s a `backgroundNonSelectionColor` read
    // from `Tree.textBackground`, ignoring both `isOpaque` and whatever
    // color the `Tree` component itself has. `ColoredTreeCellRenderer` is
    // IntelliJ's own replacement for exactly this - every built-in tree
    // (`ProjectViewTree` included) uses it, and it paints its background
    // through `RenderingUtil.getBackground(tree, selected)`, which reads
    // the real `Tree`/`Tree.Selection` colors instead of a hardcoded UI
    // resource.
    tree.cellRenderer = object : ColoredTreeCellRenderer() {
        override fun customizeCellRenderer(tree: JTree, value: Any?, selected: Boolean, expanded: Boolean, leaf: Boolean, row: Int, hasFocus: Boolean) {
            val item = treeItemOf(value) ?: return
            icon = if (item is TreeItem.Group) AllIcons.Nodes.Folder else AllIcons.FileTypes.Any_type
            append(item.displayText)
        }
    }
    return tree
}

/** Manifest tab: entries already declared in the current crate's
 * `icons.gui.toml`, grouped `manifest file -> family -> variant`, with a
 * preview pane on the right showing whatever's selected. */
private class ManifestTab(project: Project, editor: Editor, rustFilePath: String, vertical: Boolean) {
    val component: JComponent = JPanel(BorderLayout()).apply { background = UIUtil.getTreeBackground() }
    private val scope = IconBrowserScope.of(project)
    private val tree = buildTree()
    private val preview = PreviewPanel(project, scope) { item ->
        (item as? TreeItem.ManifestLeaf)?.entry?.let { IconInsertion.insert(editor, IconInsertion.manifestEntrySelector(it.family, it.size, it.variant)) }
    }

    init {
        tree.addMouseListener(treeDoubleClickToInsert(tree, editor) { item ->
            (item as? TreeItem.ManifestLeaf)?.entry?.let { IconInsertion.manifestEntrySelector(it.family, it.size, it.variant) }
        })
        tree.addTreeSelectionListener(TreeSelectionListener {
            preview.show(treeItemOf(tree.lastSelectedPathComponent).asLeafOrNull) { item ->
                (item as? TreeItem.ManifestLeaf)?.let { previewImage(it.entry.sourceFile, 128) }
            }
        })

        component.add(buildSplit(tree, preview.component, vertical), BorderLayout.CENTER)

        scope.launch {
            val manifestPath = findManifestForRustFile(rustFilePath)
            val entries = manifestPath?.let {
                when (val outcome = listManifestEntries(it)) {
                    is ListManifestEntriesOutcome.Found -> outcome.entries
                    is ListManifestEntriesOutcome.ManifestInvalid -> null
                }
            }
            withContext(Dispatchers.EDT) {
                when {
                    manifestPath == null -> component.add(JLabel("No icons.gui.toml found for this crate"), BorderLayout.NORTH)
                    entries == null -> component.add(JLabel("icons.gui.toml failed to load"), BorderLayout.NORTH)
                    else -> replaceModel(tree, buildManifestTree(File(manifestPath).name, entries))
                }
            }
        }
    }

    private fun buildManifestTree(manifestFileName: String, entries: List<ResolvedEntry>): DefaultMutableTreeNode {
        val root = node(TreeItem.Group(manifestFileName))
        for ((family, familyEntries) in entries.groupBy(ResolvedEntry::family)) {
            val familyNode = node(TreeItem.Group(family))
            familyEntries.forEach { familyNode.add(node(TreeItem.ManifestLeaf(it))) }
            root.add(familyNode)
        }
        return root
    }
}

/** Iconify tab: search-as-you-type over api.iconify.design (the same
 * `/search` endpoint iconify.design's own site uses), grouped
 * `provider -> icon name`, with the same tree+preview split as the
 * Manifest tab. Defaults to whatever collections are already cached on
 * disk before any search. */
private class IconifyTab(project: Project, editor: Editor, rustFilePath: String, vertical: Boolean) {
    val component: JComponent = JPanel(BorderLayout()).apply { background = UIUtil.getTreeBackground() }
    private val searchField = SearchTextField()
    private val scope = IconBrowserScope.of(project)
    private val cacheRoot = iconifyCacheRoot(rustFilePath)
    private val tree = buildTree()
    private val preview = PreviewPanel(project, scope) { item ->
        (item as? TreeItem.IconifyLeaf)?.let { IconInsertion.insert(editor, IconInsertion.iconifySelector(it.id)) }
    }
    private var searchJob: Job? = null

    init {
        tree.addMouseListener(treeDoubleClickToInsert(tree, editor) { item ->
            (item as? TreeItem.IconifyLeaf)?.let { IconInsertion.iconifySelector(it.id) }
        })
        tree.addTreeSelectionListener(TreeSelectionListener {
            preview.show(treeItemOf(tree.lastSelectedPathComponent).asLeafOrNull) { item ->
                (item as? TreeItem.IconifyLeaf)?.let { previewImage(ensureIconifyIconCached(cacheRoot, it.id), 128) }
            }
        })

        component.add(searchField, BorderLayout.NORTH)
        component.add(buildSplit(tree, preview.component, vertical), BorderLayout.CENTER)

        searchField.addDocumentListener(object : DocumentListener {
            override fun insertUpdate(e: DocumentEvent) = onQueryChanged()
            override fun removeUpdate(e: DocumentEvent) = onQueryChanged()
            override fun changedUpdate(e: DocumentEvent) = onQueryChanged()
        })

        loadCachedCollectionsInitially()
    }

    private fun onQueryChanged() {
        searchJob?.cancel()
        val query = searchField.text.trim()
        if (query.isEmpty()) {
            loadCachedCollectionsInitially()
            return
        }
        searchJob = scope.launch {
            delay(300)
            val results = searchIconifyIcons(query, 64u)
            withContext(Dispatchers.EDT) { replaceModel(tree, buildIconifyTree(groupByPrefix(results))) }
        }
    }

    private fun loadCachedCollectionsInitially() {
        scope.launch {
            val collectionsDir = File(findWorkspaceCacheDir(File(cacheRoot)), "_collections")
            val prefixes = collectionsDir.listFiles { f -> f.extension == "json" }?.map { it.nameWithoutExtension } ?: emptyList()
            val idsByPrefix = prefixes.associateWith { prefix -> cachedIconifyCollectionNames(cacheRoot, prefix).take(50) }
            withContext(Dispatchers.EDT) { replaceModel(tree, buildIconifyTree(idsByPrefix)) }
        }
    }

    private fun groupByPrefix(ids: List<String>): Map<String, List<String>> =
        ids.groupBy({ it.substringBefore(':') }, { it.substringAfter(':') })

    private fun buildIconifyTree(idsByPrefix: Map<String, List<String>>): DefaultMutableTreeNode {
        val root = node(TreeItem.Group("Iconify"))
        for ((prefix, names) in idsByPrefix) {
            val prefixNode = node(TreeItem.Group(prefix))
            names.forEach { name -> prefixNode.add(node(TreeItem.IconifyLeaf("$prefix:$name"))) }
            root.add(prefixNode)
        }
        return root
    }
}
