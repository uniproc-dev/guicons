package dev.guicons.ideaplugin

import com.intellij.openapi.application.EDT
import com.intellij.openapi.diagnostic.thisLogger
import com.intellij.openapi.editor.Editor
import com.intellij.openapi.editor.ScrollType
import com.intellij.openapi.editor.colors.EditorColors
import com.intellij.openapi.editor.colors.EditorColorsManager
import com.intellij.openapi.editor.event.CaretEvent
import com.intellij.openapi.editor.event.CaretListener
import com.intellij.openapi.editor.markup.HighlighterLayer
import com.intellij.openapi.editor.markup.HighlighterTargetArea
import com.intellij.openapi.editor.markup.RangeHighlighter
import com.intellij.openapi.fileEditor.FileEditorManager
import com.intellij.openapi.fileEditor.TextEditor
import com.intellij.openapi.ide.CopyPasteManager
import com.intellij.openapi.project.Project
import com.intellij.openapi.ui.ComboBox
import com.intellij.openapi.ui.popup.JBPopupFactory
import com.intellij.openapi.util.IconLoader
import com.intellij.openapi.vfs.LocalFileSystem
import com.intellij.openapi.wm.ToolWindowManager
import com.intellij.icons.AllIcons
import com.intellij.ui.ColoredTreeCellRenderer
import com.intellij.ui.JBColor
import com.intellij.ui.JBSplitter
import com.intellij.ui.SearchTextField
import com.intellij.ui.components.JBList
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
import uniffi.guicons_ffi.IconSelector
import uniffi.guicons_ffi.ListManifestEntriesOutcome
import uniffi.guicons_ffi.ResolvedEntry
import uniffi.guicons_ffi.builtinProviderNames
import uniffi.guicons_ffi.cachedIconifyCollectionNames
import uniffi.guicons_ffi.downloadIconifyCollection
import uniffi.guicons_ffi.ensureIconifyIconCached
import uniffi.guicons_ffi.entryAtOffset
import uniffi.guicons_ffi.findManifestForRustFile
import uniffi.guicons_ffi.listManifestEntries
import uniffi.guicons_ffi.macroCallAt
import uniffi.guicons_ffi.parseSelector
import uniffi.guicons_ffi.searchIconifyIcons
import uniffi.guicons_ffi.slintComponentName
import java.awt.AlphaComposite
import java.awt.BorderLayout
import java.awt.Color
import java.awt.Component
import java.awt.Cursor
import java.awt.Dimension
import java.awt.FlowLayout
import java.awt.Graphics
import java.awt.Graphics2D
import java.awt.RenderingHints
import java.awt.event.MouseAdapter
import java.awt.event.MouseEvent
import java.awt.image.BufferedImage
import java.awt.datatransfer.StringSelection
import java.io.ByteArrayInputStream
import java.io.File
import javax.imageio.ImageIO
import javax.swing.DefaultListModel
import javax.swing.Icon
import javax.swing.ImageIcon
import javax.swing.BoxLayout
import javax.swing.JComponent
import javax.swing.JLabel
import javax.swing.JList
import javax.swing.JPanel
import javax.swing.JTree
import javax.swing.ListCellRenderer
import javax.swing.ListSelectionModel
import javax.swing.SwingConstants
import javax.swing.event.DocumentEvent
import javax.swing.event.DocumentListener
import javax.swing.event.TreeSelectionListener
import javax.swing.tree.DefaultMutableTreeNode
import javax.swing.tree.DefaultTreeModel
import javax.swing.tree.TreePath
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
        const val TOOL_WINDOW_ID = "Guicons"
    }
}

/** Shared between [IconBrowserPopup] (both the floating popup and its
 * pin-to-sidebar path) and [IconBrowserToolWindowFactory] (the tool
 * window's own "currently active file" content) - same two tabs either
 * way, just built for whichever editor/file is relevant. `filePath` is
 * either a `.rs` file (the manifest it resolves against is found via
 * [findManifestForRustFile]) or a manifest file itself - the root
 * `icons.gui.toml`, or one of its `[link]`d files - in which case it
 * already *is* the thing to resolve entries against, no lookup needed.
 *
 * Also wires the reverse of what double-click/Insert does: instead of
 * picking a tree entry to write into the editor, moving the caret syncs
 * the tree selection to match whatever the caret's sitting on/in:
 * - In a `.rs` file: an already-written `icon!`/`icon_key!`/`icon_data!`
 *   call - `macroCallAt`/`parseSelector` are the exact same functions
 *   [IconDocRenderer]'s Quick Doc popup already uses to answer "what call
 *   is the caret in and what does its argument mean".
 * - In a manifest file itself: whichever entry's table the caret is
 *   physically inside, via [entryAtOffset] - the reverse direction of the
 *   `.rs` case (the manifest *is* the source of truth here, nothing to
 *   parse an `icon!` call out of).
 *
 * Either way, leaves the current selection alone when the caret isn't
 * somewhere recognized, or resolves to something neither tab's tree has
 * loaded yet - it doesn't fight the user's last explicit selection over a
 * caret that's merely passing through unrelated code. */
fun buildIconBrowserTabs(project: Project, editor: Editor, filePath: String, vertical: Boolean): JBTabbedPane {
    val isManifestFile = filePath.endsWith(".gui.toml")
    val manifestPath = if (isManifestFile) filePath else findManifestForRustFile(filePath)
    val cacheRoot = manifestPath?.let { File(it).parent } ?: File(filePath).parent

    val tabs = JBTabbedPane()
    tabs.background = UIUtil.getTreeBackground()
    val manifestTab = ManifestTab(project, editor, manifestPath, cacheRoot, vertical)
    val iconifyTab = IconifyTab(project, editor, cacheRoot, vertical)
    tabs.addTab("Manifest", manifestTab.component)
    tabs.addTab("Iconify", iconifyTab.component)

    // Updates whichever tab's tree selection matches, but never forces
    // `tabs.selectedIndex` itself - a caret move in the editor is a
    // background event from the sidebar's point of view, not a user
    // action against the browser. Switching the visible tab out from
    // under someone actively browsing the *other* one (Iconify, say,
    // while idly moving the caret through unrelated Rust code) would be
    // fighting their own last explicit choice, the same reason
    // `showForCurrentFile` doesn't blank the browser out for an unrelated
    // file. The match still gets selected in its own tab's tree either
    // way, so switching to it manually shows the right thing already
    // highlighted.
    editor.caretModel.addCaretListener(object : CaretListener {
        override fun caretPositionChanged(event: CaretEvent) {
            val offset = event.editor.caretModel.offset
            if (isManifestFile) {
                val entry = manifestPath?.let { entryAtOffset(it, filePath, offset.toUInt()) } ?: return
                manifestTab.selectEntryMatching(entry.family, entry.size?.toInt(), entry.variant)
                return
            }
            val site = macroCallAt(event.editor.document.text, offset.toUInt()) ?: return
            when (val selector = parseSelector(site.argText)) {
                is IconSelector.FamilyVariant -> manifestTab.selectEntryMatching(selector.family, selector.size?.toInt(), selector.variant)
                is IconSelector.Iconify -> iconifyTab.selectIconIfPresent(selector.id)
                null -> Unit
            }
        }
    })

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

/** [ResolvedEntry.sourceFile] is only ever set for a `file`-sourced entry
 * - an `iconify`-sourced one (`Source: iconify \`prefix:name\`` in the
 * details panel) has no local asset to read at all until it's actually
 * fetched, same as an id the user found by browsing/searching the
 * Iconify tab. Previously this just fell through to `previewImage(null,
 * ...)` and stayed blank forever for every iconify-sourced manifest
 * entry - not a network/rendering bug, the fetch was simply never
 * attempted. */
private suspend fun manifestLeafPreviewImage(entry: ResolvedEntry, cacheRoot: String): BufferedImage? {
    entry.sourceFile?.let { return previewImage(it, 256) }
    val iconifyId = entry.iconifyId ?: return null
    return previewImage(ensureIconifyIconCached(cacheRoot, iconifyId), 256)
}

private fun replaceModel(tree: Tree, root: DefaultMutableTreeNode) {
    tree.model = DefaultTreeModel(root)
    for (i in 0 until tree.rowCount) tree.expandRow(i)
}

/** Fills whatever space its container gives it (no fixed/preferred size
 * of its own - unlike the fixed `CARD_SIZE` this used to have) but only
 * ever paints a square within that space, sized to the smaller of the
 * two dimensions and centered - so it grows to use the full width of a
 * wide popup or a narrow sidebar alike, without ever stretching into a
 * non-square rectangle. Colored to contrast with the icon's *own* pixels
 * ([IconPreviewCard.contrastingCardColor]) rather than a fixed theme
 * color - a raw image floating directly on the panel background looks
 * like a rendering glitch rather than a preview, and a fixed card color
 * can still end up the same shade as the icon itself (e.g. an SVG with
 * no fill set, which just renders solid black). */
private class IconCard : JPanel() {
    var image: BufferedImage? = null
        set(value) {
            field = value
            cardColor = IconPreviewCard.contrastingCardColor(value)
            repaint()
        }
    private var cardColor: Color = IconPreviewCard.contrastingCardColor(null)

    init {
        isOpaque = false
    }

    override fun paintComponent(g: Graphics) {
        super.paintComponent(g)
        val size = minOf(width, height)
        if (size <= 0) return
        val x = (width - size) / 2
        val y = (height - size) / 2
        val g2 = g.create() as Graphics2D
        try {
            g2.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON)
            g2.color = cardColor
            g2.fillRoundRect(x, y, size, size, ARC, ARC)
            image?.let { drawScaledToFit(g2, it, x, y, size) }
        } finally {
            g2.dispose()
        }
    }

    /** Scales `image` to [IMAGE_FRACTION] of the card's own square size
     * (up *or* down - the source bitmap is a fixed raster from
     * [previewImage], so without this the icon would stay pinned at its
     * original pixel size and visibly ignore the card growing or
     * shrinking around it, the opposite of the card itself, which really
     * does track the available space). */
    private fun drawScaledToFit(g2: Graphics2D, image: BufferedImage, cardX: Int, cardY: Int, cardSize: Int) {
        val target = (cardSize * IMAGE_FRACTION).toInt().coerceAtLeast(1)
        val scale = minOf(target.toDouble() / image.width, target.toDouble() / image.height)
        val w = (image.width * scale).toInt().coerceAtLeast(1)
        val h = (image.height * scale).toInt().coerceAtLeast(1)
        g2.setRenderingHint(RenderingHints.KEY_INTERPOLATION, RenderingHints.VALUE_INTERPOLATION_BILINEAR)
        g2.drawImage(image, cardX + (cardSize - w) / 2, cardY + (cardSize - h) / 2, w, h, null)
    }

    companion object {
        private const val ARC = 16
        private const val IMAGE_FRACTION = 0.7
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

/** One highlighter per editor at a time - each new selection replaces
 * whatever this same editor was showing before, rather than accumulating
 * a permanent marker per entry ever browsed to. */
private val activeManifestHighlighters = HashMap<Editor, RangeHighlighter>()

/** Marks `entry`'s own span in whichever editor already has its
 * declaring file open - the root manifest, or the specific `[link]`d
 * file this particular entry actually lives in - without opening it if
 * it isn't (unlike [openFile]/the "Declared in" link, this fires on
 * every tree selection change, not an explicit click - forcing a new
 * editor tab open just from browsing the tree would be far too
 * aggressive).
 *
 * A real `selectionModel` selection (what this used to do) reads as the
 * user's own manual text selection, not a passive "here's what this
 * points at" marker, and doesn't belong moving on every tree click - a
 * `RangeHighlighter` with the platform's own search-result-style
 * attributes is the same visual language `EditorColors.SEARCH_RESULT_ATTRIBUTES`
 * gives every other "point at this range" feature in the IDE, and it
 * doesn't touch the caret or steal focus from the tree either. */
private fun highlightEntryIfFileOpen(project: Project, entry: ResolvedEntry) {
    val fileManager = FileEditorManager.getInstance(project)
    val file = LocalFileSystem.getInstance().refreshAndFindFileByPath(stripWindowsVerbatimPrefix(entry.declaredInFilePath)) ?: return
    if (!fileManager.isFileOpen(file)) return
    val editor = fileManager.getEditors(file).filterIsInstance<TextEditor>().firstOrNull()?.editor ?: return
    val length = editor.document.textLength
    val start = entry.declaredInSpanStart.toInt().coerceIn(0, length)
    val end = entry.declaredInSpanEnd.toInt().coerceIn(start, length)

    activeManifestHighlighters.remove(editor)?.let { editor.markupModel.removeHighlighter(it) }
    val attributes = EditorColorsManager.getInstance().globalScheme.getAttributes(EditorColors.SEARCH_RESULT_ATTRIBUTES)
    val highlighter = editor.markupModel.addRangeHighlighter(start, end, HighlighterLayer.SELECTION - 1, attributes, HighlighterTargetArea.EXACT_RANGE)
    activeManifestHighlighters[editor] = highlighter

    editor.scrollingModel.scrollTo(editor.offsetToLogicalPosition(start), ScrollType.MAKE_VISIBLE)
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

private fun loadIcon(resourcePath: String): Icon = IconLoader.getIcon(resourcePath, IconBrowserPopup::class.java)

/** Borderless, background-less icon button for [PreviewPanel]'s header -
 * a plain `JButton` (even fully stripped of border/content-area-fill via
 * `isBorderPainted`/`isContentAreaFilled`) never gave real hover/press
 * feedback once those were off, since that's exactly what they're
 * responsible for painting. Paints its own rounded hover/pressed
 * highlight instead, dims the icon when disabled, and only fires
 * `onClick` on a release that's still inside the button (the standard
 * "click" contract - a press-drag-out-release shouldn't trigger it). */
private class HoverIconButton(private val icon: Icon, tooltip: String, private val onClick: () -> Unit) : JComponent() {
    private var hovered = false
    private var pressed = false

    init {
        toolTipText = tooltip
        cursor = Cursor.getPredefinedCursor(Cursor.HAND_CURSOR)
        preferredSize = Dimension(icon.iconWidth + PADDING * 2, icon.iconHeight + PADDING * 2)
        addMouseListener(object : MouseAdapter() {
            override fun mouseEntered(e: MouseEvent) {
                if (!isEnabled) return
                hovered = true
                repaint()
            }

            override fun mouseExited(e: MouseEvent) {
                hovered = false
                pressed = false
                repaint()
            }

            override fun mousePressed(e: MouseEvent) {
                if (!isEnabled) return
                pressed = true
                repaint()
            }

            override fun mouseReleased(e: MouseEvent) {
                val wasPressed = pressed
                pressed = false
                repaint()
                if (isEnabled && wasPressed && contains(e.point)) onClick()
            }
        })
    }

    override fun paintComponent(g: Graphics) {
        super.paintComponent(g)
        val g2 = g.create() as Graphics2D
        try {
            g2.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON)
            if (isEnabled) {
                if (pressed) {
                    g2.color = PRESSED_COLOR
                    g2.fillRoundRect(0, 0, width, height, ARC, ARC)
                } else if (hovered) {
                    g2.color = HOVER_COLOR
                    g2.fillRoundRect(0, 0, width, height, ARC, ARC)
                }
            } else {
                g2.composite = AlphaComposite.getInstance(AlphaComposite.SRC_OVER, 0.4f)
            }
            icon.paintIcon(this, g2, (width - icon.iconWidth) / 2, (height - icon.iconHeight) / 2)
        } finally {
            g2.dispose()
        }
    }

    companion object {
        private const val PADDING = 5
        private const val ARC = 6
        private val HOVER_COLOR = JBColor(Color(0, 0, 0, 25), Color(255, 255, 255, 25))
        private val PRESSED_COLOR = JBColor(Color(0, 0, 0, 45), Color(255, 255, 255, 45))
    }
}

private fun iconButton(icon: Icon, tooltip: String, onClick: () -> Unit): HoverIconButton = HoverIconButton(icon, tooltip, onClick)

private fun copyToClipboard(text: String) {
    CopyPasteManager.getInstance().setContents(StringSelection(text))
}

/** What [PreviewPanel]'s copy-format combo offers for `item` - a handful
 * of ready-to-paste representations, since which one's actually wanted
 * depends entirely on where it's being pasted (a different file/project
 * than the browser's own `editor`, a doc, chat, wherever `Insert`
 * wouldn't reach). `icon!(...)` uses the exact selector syntax
 * [IconInsertion.insert] itself writes; `guicons::icon!(...)` is the same
 * but fully qualified, for code that hasn't (or can't) `use`d the macro.
 * A `ManifestLeaf` additionally offers the Slint component `guicons-build`
 * generates for it ([slintComponentName]) - meaningless for a bare
 * [TreeItem.IconifyLeaf], which is just something found by browsing/
 * searching iconify.design and isn't in any manifest yet, so has no
 * corresponding on-disk asset or generated component to instantiate. A
 * `Group` never reaches the preview at all (see `asLeafOrNull`), so isn't
 * handled here. */
private fun copyFormatsFor(item: TreeItem): List<Pair<String, String>> = when (item) {
    is TreeItem.Group -> emptyList()
    is TreeItem.IconifyLeaf -> {
        val selector = IconInsertion.iconifySelector(item.id)
        listOf(
            "Iconify id" to item.id,
            "icon!(...)" to "icon!($selector)",
            "guicons::icon!(...)" to "guicons::icon!($selector)",
        )
    }
    is TreeItem.ManifestLeaf -> {
        val entry = item.entry
        val selector = IconInsertion.manifestEntrySelector(entry.family, entry.size, entry.variant)
        listOf(
            "Name" to item.displayText,
            "icon!(...)" to "icon!($selector)",
            "guicons::icon!(...)" to "guicons::icon!($selector)",
            "Slint (${slintComponentName(entry.key)} {})" to "${slintComponentName(entry.key)} {}",
        )
    }
}

/** Right-hand side of the split: title + copy/insert icon buttons along
 * the top, a square preview card filling the middle, a copy-format picker
 * right below it, and details at the bottom - for whatever leaf is
 * currently selected in the tree. Double-click on the tree does the same
 * thing the insert button does; a selection-only click (needed anyway to
 * *see* the preview) doesn't also insert. */
private class PreviewPanel(private val project: Project, private val scope: CoroutineScope, private val onInsert: (TreeItem) -> Unit) {
    val component: JComponent = JPanel(BorderLayout()).apply { background = UIUtil.getTreeBackground() }
    private val iconCard = IconCard()
    private val titleLabel = JLabel("")
    private val detailsPanel = JPanel()
    private val formatCombo = ComboBox<String>()
    private val copyButton = iconButton(loadIcon("/icons/previewCopy.svg"), "Copy") { copySelectedFormat() }
    private val insertButton = iconButton(loadIcon("/icons/previewInsert.svg"), "Insert") { current?.let(onInsert) }
    private var current: TreeItem? = null
    private var copyFormats: List<Pair<String, String>> = emptyList()
    private var loadJob: Job? = null

    init {
        component.border = JBUI.Borders.empty(20)
        titleLabel.font = JBFont.label().asBold()
        detailsPanel.layout = BoxLayout(detailsPanel, BoxLayout.Y_AXIS)
        detailsPanel.isOpaque = false
        detailsPanel.border = JBUI.Borders.emptyTop(8)
        copyButton.isEnabled = false
        insertButton.isEnabled = false

        val actionsRow = JPanel(FlowLayout(FlowLayout.RIGHT, 6, 0))
        actionsRow.isOpaque = false
        actionsRow.add(copyButton)
        actionsRow.add(insertButton)

        val titleRow = JPanel(BorderLayout())
        titleRow.isOpaque = false
        titleRow.border = JBUI.Borders.emptyBottom(8)
        titleRow.add(titleLabel, BorderLayout.CENTER)
        titleRow.add(actionsRow, BorderLayout.EAST)

        val formatRow = JPanel(FlowLayout(FlowLayout.LEFT, 0, 0))
        formatRow.isOpaque = false
        formatRow.border = JBUI.Borders.emptyTop(8)
        formatRow.add(formatCombo)
        formatCombo.isVisible = false

        // Below the whole preview block (card + details), not sandwiched
        // between them - the picker is a secondary tool for grabbing a
        // reference in a different format, not something that belongs in
        // the middle of the "what is this icon" reading order.
        val bottomPanel = JPanel(BorderLayout())
        bottomPanel.isOpaque = false
        bottomPanel.add(detailsPanel, BorderLayout.NORTH)
        bottomPanel.add(formatRow, BorderLayout.SOUTH)

        component.add(titleRow, BorderLayout.NORTH)
        component.add(iconCard, BorderLayout.CENTER)
        component.add(bottomPanel, BorderLayout.SOUTH)
        showNothing()
    }

    private fun copySelectedFormat() {
        val index = formatCombo.selectedIndex
        if (index in copyFormats.indices) copyToClipboard(copyFormats[index].second)
    }

    fun show(item: TreeItem?, resolveImage: suspend (TreeItem) -> BufferedImage?) {
        loadJob?.cancel()
        current = item
        if (item == null) {
            showNothing()
            return
        }
        copyButton.isEnabled = true
        insertButton.isEnabled = true
        iconCard.image = null
        titleLabel.text = item.displayText
        copyFormats = copyFormatsFor(item)
        formatCombo.removeAllItems()
        copyFormats.forEach { (label, _) -> formatCombo.addItem(label) }
        formatCombo.isVisible = copyFormats.isNotEmpty()
        detailsPanel.removeAll()
        detailRows(project, item).forEach(detailsPanel::add)
        detailsPanel.revalidate()
        detailsPanel.repaint()
        loadJob = scope.launch {
            val image = try {
                resolveImage(item)
            } catch (e: Exception) {
                // Was silently swallowed before - a failed download/render
                // just left the card blank forever with nothing in
                // idea.log to say why. Log it and keep showing nothing,
                // rather than pretending "null" always means "no preview
                // available".
                thisLogger().warn("Failed to resolve preview image for ${item.displayText}", e)
                null
            }
            withContext(Dispatchers.EDT) {
                if (current === item) iconCard.image = image
            }
        }
    }

    private fun showNothing() {
        iconCard.image = null
        titleLabel.text = "Select an icon to preview"
        copyFormats = emptyList()
        formatCombo.removeAllItems()
        formatCombo.isVisible = false
        detailsPanel.removeAll()
        detailsPanel.revalidate()
        detailsPanel.repaint()
        copyButton.isEnabled = false
        insertButton.isEnabled = false
    }
}

/** The tree- or grid-side component and preview, separated by nothing
 * more than `JBSplitter`'s own thin divider line - horizontal (list left,
 * preview right) for a wide, short floating popup; vertical (preview on
 * top, list below - the order that reads naturally top-to-bottom for a
 * narrow, tall sidebar) when pinned into a tool window.
 *
 * Every prior attempt at this fought the tree itself - guessing a color
 * name to paint it with, or forcing `isOpaque` one way or the other.
 * `Help > Diagnostic Tools > UI Inspector`'d against the *platform's own*
 * project tree settled it: `ProjectViewTree` doesn't override its
 * background or opacity at all - it's a plain, untouched
 * `com.intellij.ui.treeStructure.Tree`, same base class as [ManifestTab]'s
 * tree, and it blends in because everything built *around* it (there, the
 * Project tool window's own panels) is colored to match the tree's own
 * natural `Tree.background`, never the other way round. So here: `side`
 * and its scroll pane are left completely alone (background-wise), and
 * [buildIconBrowserTabs]/[PreviewPanel] paint themselves with
 * `UIUtil.getTreeBackground()` instead. `configureScroll` is a hook onto
 * the scroll pane [buildSplit] creates around `side` - [IconifyTab]'s grid
 * uses it to wire up infinite scroll against the actual scrollbar, which
 * doesn't exist as a component of its own until this function creates it. */
private fun buildSplit(side: JComponent, preview: JComponent, vertical: Boolean, configureScroll: (JBScrollPane) -> Unit = {}): JBSplitter {
    val splitter = JBSplitter(vertical, if (vertical) 0.45f else 0.55f)
    splitter.dividerWidth = 1
    splitter.setShowDividerControls(false)
    splitter.background = UIUtil.getTreeBackground()
    side.border = JBUI.Borders.empty(8, 4)
    val sideScroll = JBScrollPane(side)
    // A visible seam between preview and the list - the divider itself
    // paints in the same now-uniform background, so without an explicit
    // line here the two sides would run together with no separation at
    // all. Drawn on whichever side actually touches the preview, since
    // that's the only edge JBSplitter's thin divider runs along.
    sideScroll.border = if (vertical) JBUI.Borders.customLineTop(JBColor.border()) else JBUI.Borders.customLineRight(JBColor.border())
    configureScroll(sideScroll)
    if (vertical) {
        splitter.firstComponent = preview
        splitter.secondComponent = sideScroll
    } else {
        splitter.firstComponent = sideScroll
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

/** Finds the first leaf in `tree` matching `predicate` and, if there is
 * one, selects and scrolls to it - the caret-driven "what's under the
 * cursor right now" sync ([buildIconBrowserTabs]) shares this against
 * both tabs' trees rather than duplicating the walk. `false` (nothing
 * selected) both when nothing matches and when `tree` hasn't finished
 * loading yet - either way there's nothing to point at. */
private fun selectMatchingLeaf(tree: Tree, predicate: (TreeItem) -> Boolean): Boolean {
    val root = tree.model.root as? DefaultMutableTreeNode ?: return false
    val found = findLeafNode(root, predicate) ?: return false
    val path = TreePath(found.path)
    tree.selectionPath = path
    tree.scrollPathToVisible(path)
    return true
}

private fun findLeafNode(node: DefaultMutableTreeNode, predicate: (TreeItem) -> Boolean): DefaultMutableTreeNode? {
    val item = treeItemOf(node).asLeafOrNull
    if (item != null && predicate(item)) return node
    for (i in 0 until node.childCount) {
        val child = node.getChildAt(i) as DefaultMutableTreeNode
        findLeafNode(child, predicate)?.let { return it }
    }
    return null
}

/** Manifest tab: entries already declared in `manifestPath` (the current
 * crate's `icons.gui.toml`, resolved once by the caller - see
 * [buildIconBrowserTabs] - rather than re-derived here from a `.rs`
 * file, since the caller might not have one at all), grouped
 * `manifest file -> family -> variant`, with a preview pane on the right
 * showing whatever's selected. */
private class ManifestTab(project: Project, editor: Editor, manifestPath: String?, cacheRoot: String, vertical: Boolean) {
    val component: JComponent = JPanel(BorderLayout()).apply { background = UIUtil.getTreeBackground() }
    private val scope = IconBrowserScope.of(project)
    private val tree = buildTree()
    private val preview = PreviewPanel(project, scope) { item ->
        (item as? TreeItem.ManifestLeaf)?.entry?.let { IconInsertion.insert(editor, IconInsertion.manifestEntrySelector(it.family, it.size, it.variant)) }
    }

    /** `true` if `family`/`size`/`variant` (whatever the caret is
     * currently inside an `icon!(...)` call for, see [buildIconBrowserTabs])
     * matches an already-loaded entry in this tab's tree. */
    fun selectEntryMatching(family: String, size: Int?, variant: String?): Boolean =
        selectMatchingLeaf(tree) { item ->
            item is TreeItem.ManifestLeaf && item.entry.family == family && item.entry.size?.toInt() == size && item.entry.variant == variant
        }

    init {
        tree.addMouseListener(treeDoubleClickToInsert(tree, editor) { item ->
            (item as? TreeItem.ManifestLeaf)?.entry?.let { IconInsertion.manifestEntrySelector(it.family, it.size, it.variant) }
        })
        tree.addTreeSelectionListener(TreeSelectionListener {
            val item = treeItemOf(tree.lastSelectedPathComponent).asLeafOrNull
            preview.show(item) { i -> (i as? TreeItem.ManifestLeaf)?.let { manifestLeafPreviewImage(it.entry, cacheRoot) } }
            (item as? TreeItem.ManifestLeaf)?.entry?.let { highlightEntryIfFileOpen(project, it) }
        })

        component.add(buildSplit(tree, preview.component, vertical), BorderLayout.CENTER)

        scope.launch {
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

    /** Grouped by *declaring file* first, then family - a `[link]
     * includes = [...]`d file's entries get their own subtree under their
     * own display path (`icons/extra.gui.toml`), not silently flattened
     * into the same family groups as the root manifest's own entries.
     * `list_manifest_entries` already resolves the whole include graph
     * into one flat list (that's the entire point of `[link]`), so this
     * is purely a presentation grouping, not a second parse. */
    private fun buildManifestTree(manifestFileName: String, entries: List<ResolvedEntry>): DefaultMutableTreeNode {
        val root = node(TreeItem.Group(manifestFileName))
        for ((declaredInFile, fileEntries) in entries.groupBy(ResolvedEntry::declaredInFile)) {
            val fileNode = node(TreeItem.Group(declaredInFile))
            for ((family, familyEntries) in fileEntries.groupBy(ResolvedEntry::family)) {
                val familyNode = node(TreeItem.Group(family))
                familyEntries.forEach { familyNode.add(node(TreeItem.ManifestLeaf(it))) }
                fileNode.add(familyNode)
            }
            root.add(fileNode)
        }
        return root
    }
}

/** A single cell in [IconifyTab]'s grid - a small contrasting-background
 * card (same [IconPreviewCard] treatment the big preview uses, baked into
 * a single composited bitmap rather than live-painted - a grid can have
 * far more cells than the one big preview ever needs to redraw at once)
 * with the id fragment below it. Reused across
 * `getListCellRendererComponent` calls (the standard `JList` renderer
 * pattern - one instance configured and returned each time, not a fresh
 * component per cell) with its own small thumbnail cache, since
 * [previewImage]/[ensureIconifyIconCached] are each too expensive to run
 * synchronously inside a paint call for every visible cell on every
 * repaint. A cell whose icon isn't cached yet kicks off a fetch and
 * repaints the whole list once it lands - cheap enough for the handful of
 * cells actually visible at once, and simpler than tracking individual
 * cell rects. */
private class IconGridCellRenderer(private val cacheRoot: String, private val scope: CoroutineScope) : JPanel(BorderLayout()), ListCellRenderer<TreeItem.IconifyLeaf> {
    private val iconLabel = JLabel("", SwingConstants.CENTER)
    private val nameLabel = JLabel("", SwingConstants.CENTER)
    private val thumbnails = HashMap<String, Icon?>()
    private val loading = HashSet<String>()
    private var owner: JList<out TreeItem.IconifyLeaf>? = null

    init {
        isOpaque = true
        border = JBUI.Borders.empty(4)
        nameLabel.font = JBUI.Fonts.smallFont()
        add(iconLabel, BorderLayout.CENTER)
        add(nameLabel, BorderLayout.SOUTH)
    }

    override fun getListCellRendererComponent(
        list: JList<out TreeItem.IconifyLeaf>,
        value: TreeItem.IconifyLeaf,
        index: Int,
        isSelected: Boolean,
        cellHasFocus: Boolean,
    ): Component {
        owner = list
        background = if (isSelected) list.selectionBackground else UIUtil.getTreeBackground()
        nameLabel.foreground = if (isSelected) list.selectionForeground else UIUtil.getTreeForeground()
        nameLabel.text = value.displayText
        iconLabel.icon = thumbnails[value.id]
        maybeLoad(value)
        return this
    }

    private fun maybeLoad(item: TreeItem.IconifyLeaf) {
        if (thumbnails.containsKey(item.id) || !loading.add(item.id)) return
        scope.launch {
            val image = previewImage(ensureIconifyIconCached(cacheRoot, item.id), CARD_ICON_PX)
            val cardIcon = image?.let {
                val cardBytes = IconPreviewCard.renderCardPng(it, CARD_SIZE_PX, CARD_ARC_PX)
                ImageIO.read(ByteArrayInputStream(cardBytes))?.let(::ImageIcon)
            }
            withContext(Dispatchers.EDT) {
                thumbnails[item.id] = cardIcon
                loading.remove(item.id)
                owner?.repaint()
            }
        }
    }

    companion object {
        // A grid cell fetches a much smaller raster than the one big
        // preview card ([manifestLeafPreviewImage]/the Iconify tab's own
        // single-selection preview both request 256px) - dozens of these
        // can be visible/loading at once, so keeping each one cheap to
        // fetch and render matters here in a way it doesn't for a single
        // preview.
        private const val CARD_ICON_PX = 28
        const val CARD_SIZE_PX = 40
        private const val CARD_ARC_PX = 8
    }
}

private fun buildIconGrid(cacheRoot: String, scope: CoroutineScope): JBList<TreeItem.IconifyLeaf> {
    val list = JBList(DefaultListModel<TreeItem.IconifyLeaf>())
    list.layoutOrientation = JList.HORIZONTAL_WRAP
    list.visibleRowCount = 0
    // Cell must be taller than [IconGridCellRenderer.CARD_SIZE_PX] to
    // leave room for the name label below the card without squeezing/
    // clipping it - that mismatch (a fixed cell size with no headroom for
    // the label) was the actual cause of icons looking cut off/squashed
    // into a non-square area, not the card itself.
    list.fixedCellWidth = JBUI.scale(64)
    list.fixedCellHeight = JBUI.scale(IconGridCellRenderer.CARD_SIZE_PX + 28)
    list.selectionMode = ListSelectionModel.SINGLE_SELECTION
    list.cellRenderer = IconGridCellRenderer(cacheRoot, scope)
    return list
}

private fun JBList<TreeItem.IconifyLeaf>.setItems(items: List<TreeItem.IconifyLeaf>) {
    val listModel = model as DefaultListModel<TreeItem.IconifyLeaf>
    listModel.clear()
    items.forEach(listModel::addElement)
}

/** `true` only if `id` is already a cell in this grid's *current* items
 * (whatever's revealed/searched so far) - doesn't trigger a fetch for it,
 * since the caret sync this backs ([buildIconBrowserTabs]) fires on every
 * caret move and a network round-trip per keystroke-adjacent move would
 * be its own problem. */
private fun JBList<TreeItem.IconifyLeaf>.selectIfPresent(id: String): Boolean {
    val listModel = model as DefaultListModel<TreeItem.IconifyLeaf>
    for (i in 0 until listModel.size()) {
        if (listModel[i].id == id) {
            selectedIndex = i
            ensureIndexIsVisible(i)
            return true
        }
    }
    return false
}

/** Fires `onNearBottom` once whenever the scrollbar gets within
 * [THRESHOLD_PX] of its max, for [IconifyTab]'s infinite scroll - not
 * `valueChanged` on every scroll tick, which would re-trigger constantly
 * while the user's still scrolling through content that's already loaded. */
private fun JBScrollPane.onScrolledNearBottom(onNearBottom: () -> Unit) {
    verticalScrollBar.addAdjustmentListener { e ->
        val bar = e.adjustable
        if (bar.value + bar.visibleAmount >= bar.maximum - THRESHOLD_PX) onNearBottom()
    }
}

private const val THRESHOLD_PX = 400

/** Iconify tab: a provider picker above a grid of icons - browsing (the
 * provider dropdown) or search-as-you-type (the same `/search` endpoint
 * iconify.design's own site uses) fill the same grid, revealed in batches
 * as the user scrolls rather than paginated - there's no natural "page"
 * concept in a grid the way there is in a numbered list, and the full
 * name list for a browsed provider is already sitting in memory once
 * cached (see `guicons_net::cached_collection_names`), so "load more" is
 * really just "show more of what's already known" rather than an actual
 * fetch, except while actively searching (see [onNearBottom]). */
private class IconifyTab(project: Project, editor: Editor, private val cacheRoot: String, vertical: Boolean) {
    val component: JComponent = JPanel(BorderLayout()).apply { background = UIUtil.getTreeBackground() }
    private val searchField = SearchTextField()
    private val providerCombo = ComboBox<String>()
    private val scope = IconBrowserScope.of(project)
    private val grid = buildIconGrid(cacheRoot, scope)
    private val preview = PreviewPanel(project, scope) { item ->
        (item as? TreeItem.IconifyLeaf)?.let { IconInsertion.insert(editor, IconInsertion.iconifySelector(it.id)) }
    }
    private var searchJob: Job? = null

    /** All ids currently known for the active provider/search, in
     * `"prefix:name"` form - the grid only ever shows `allIds.take(revealed)`
     * of this, growing as the user scrolls ([revealMore]). */
    private var allIds: List<String> = emptyList()
    private var revealed = 0

    /** Non-null while actively searching - `null` means the provider
     * dropdown is in charge instead. Distinct from "search field is
     * empty" because clearing it needs to fall back to whatever provider
     * is selected, not to an empty grid. */
    private var activeQuery: String? = null
    private var searchLimit = INITIAL_SEARCH_LIMIT

    fun selectIconIfPresent(id: String): Boolean = grid.selectIfPresent(id)

    init {
        grid.addListSelectionListener { e ->
            if (e.valueIsAdjusting) return@addListSelectionListener
            preview.show(grid.selectedValue) { item ->
                (item as? TreeItem.IconifyLeaf)?.let { previewImage(ensureIconifyIconCached(cacheRoot, it.id), 256) }
            }
        }
        grid.addMouseListener(object : MouseAdapter() {
            override fun mouseClicked(e: MouseEvent) {
                if (e.clickCount != 2) return
                val index = grid.locationToIndex(e.point)
                if (index < 0) return
                IconInsertion.insert(editor, IconInsertion.iconifySelector((grid.model as DefaultListModel<TreeItem.IconifyLeaf>)[index].id))
            }
        })

        val topRow = JPanel(BorderLayout())
        topRow.isOpaque = false
        topRow.add(providerCombo, BorderLayout.WEST)
        topRow.add(searchField, BorderLayout.CENTER)

        component.add(topRow, BorderLayout.NORTH)
        component.add(buildSplit(grid, preview.component, vertical) { it.onScrolledNearBottom(::onNearBottom) }, BorderLayout.CENTER)

        searchField.addDocumentListener(object : DocumentListener {
            override fun insertUpdate(e: DocumentEvent) = onQueryChanged()
            override fun removeUpdate(e: DocumentEvent) = onQueryChanged()
            override fun changedUpdate(e: DocumentEvent) = onQueryChanged()
        })
        providerCombo.addActionListener {
            if (activeQuery == null) (providerCombo.selectedItem as? String)?.let { loadProvider(it) }
        }

        loadProviderList()
    }

    /** Populates the dropdown with guicons' built-in provider names
     * (always available, no network needed) plus whatever's already
     * cached on disk from earlier browsing - a curated, on-brand default
     * rather than iconify.design's full, unfiltered collection list
     * (hundreds of entries, and would need its own network round-trip
     * this tab has never needed before just to show a picker). */
    private fun loadProviderList() {
        scope.launch {
            val cachedPrefixes = File(findWorkspaceCacheDir(File(cacheRoot)), "_collections")
                .listFiles { f -> f.extension == "json" }
                ?.map { it.nameWithoutExtension }
                ?: emptyList()
            val providers = (builtinProviderNames() + cachedPrefixes).distinct().sorted()
            withContext(Dispatchers.EDT) {
                providers.forEach(providerCombo::addItem)
                providers.firstOrNull()?.let { loadProvider(it) }
            }
        }
    }

    private fun loadProvider(provider: String) {
        scope.launch {
            downloadIconifyCollection(cacheRoot, provider)
            val names = cachedIconifyCollectionNames(cacheRoot, provider)
            allIds = names.map { "$provider:$it" }
            revealed = 0
            withContext(Dispatchers.EDT) { revealMore() }
        }
    }

    private fun onQueryChanged() {
        searchJob?.cancel()
        val query = searchField.text.trim()
        if (query.isEmpty()) {
            activeQuery = null
            (providerCombo.selectedItem as? String)?.let { loadProvider(it) }
            return
        }
        activeQuery = query
        searchLimit = INITIAL_SEARCH_LIMIT
        searchJob = scope.launch {
            delay(300)
            runSearch()
        }
    }

    private suspend fun runSearch() {
        val query = activeQuery ?: return
        allIds = searchIconifyIcons(query, searchLimit.toUInt())
        revealed = 0
        withContext(Dispatchers.EDT) { revealMore() }
    }

    /** Browsing: reveal the next batch of the already-known full name
     * list. Searching: iconify's `/search` only ever returns up to
     * `limit` results with no separate offset/cursor, so "more" means
     * re-running the same search with a bigger `limit` - simple, if not
     * maximally efficient, and this is a best-effort browse UI, not a
     * paginated data grid. */
    private fun onNearBottom() {
        val query = activeQuery
        if (query != null) {
            if (allIds.size.toUInt() < searchLimit) return // fewer results than asked for - nothing more exists
            searchLimit += SEARCH_BATCH
            searchJob?.cancel()
            searchJob = scope.launch { runSearch() }
        } else {
            revealMore()
        }
    }

    private fun revealMore() {
        val next = (revealed + REVEAL_BATCH).coerceAtMost(allIds.size)
        if (next == revealed) return
        revealed = next
        grid.setItems(allIds.take(revealed).map { TreeItem.IconifyLeaf(it) })
    }

    companion object {
        private const val REVEAL_BATCH = 60
        private const val INITIAL_SEARCH_LIMIT = 60u
        private const val SEARCH_BATCH = 60u
    }
}
