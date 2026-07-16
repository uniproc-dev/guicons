package dev.guicons.ideaplugin

import com.intellij.openapi.command.WriteCommandAction
import com.intellij.openapi.editor.Editor
import uniffi.guicons_ffi.macroCallAt

/**
 * Inserts an icon reference at the caret - shared by both
 * [IconBrowserPopup] tabs (manifest entries and Iconify search results).
 *
 * Context-aware: if the caret is already inside a guicons macro call's
 * argument, replaces just that argument (matching what a user would type
 * by hand - `family.variant` or `"provider:name"`); otherwise inserts a
 * full `icon!(...)` call at the caret.
 */
object IconInsertion {
    /** `family.variant`/`family.size.variant` - a valid bare path selector, no quotes. */
    fun manifestEntrySelector(family: String, size: UShort?, variant: String?): String =
        listOfNotNull(family, size?.toString(), variant).joinToString(".")

    /** `"provider:name"` - iconify ids always go through the quoted-string grammar. */
    fun iconifySelector(id: String): String = "\"$id\""

    fun insert(editor: Editor, selectorText: String) {
        val document = editor.document
        val offset = editor.caretModel.offset
        val site = macroCallAt(document.text, offset.toUInt())

        WriteCommandAction.runWriteCommandAction(editor.project, "Insert Guicons Icon", null, {
            if (site != null) {
                document.replaceString(site.argStart.toInt(), site.argEnd.toInt(), selectorText)
                editor.caretModel.moveToOffset(site.argStart.toInt() + selectorText.length)
            } else {
                val call = "icon!($selectorText)"
                document.insertString(offset, call)
                editor.caretModel.moveToOffset(offset + call.length)
            }
        })
    }
}
