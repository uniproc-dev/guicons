package dev.guicons.ideaplugin

import org.apache.batik.transcoder.TranscoderInput
import org.apache.batik.transcoder.TranscoderOutput
import org.apache.batik.transcoder.image.PNGTranscoder
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.FileInputStream

/** Rasterizes an SVG file to PNG bytes at a fixed square size, for
 * embedding in the doc popup - see [GuiconsDocumentationProvider]. */
object SvgRenderer {
    fun renderToPngBytes(svgFile: File, sizePx: Int): ByteArray =
        FileInputStream(svgFile).use { renderToPngBytes(it, sizePx) }

    /** Same rasterization, from raw SVG bytes rather than a file on disk -
     * for previewing an icon fetched purely in-memory (see
     * `fetchIconifyIconPreview`), which is never written to disk in the
     * first place. */
    fun renderToPngBytes(svgBytes: ByteArray, sizePx: Int): ByteArray = renderToPngBytes(ByteArrayInputStream(svgBytes), sizePx)

    private fun renderToPngBytes(svgInput: java.io.InputStream, sizePx: Int): ByteArray {
        val transcoder = PNGTranscoder()
        transcoder.addTranscodingHint(PNGTranscoder.KEY_WIDTH, sizePx.toFloat())
        transcoder.addTranscodingHint(PNGTranscoder.KEY_HEIGHT, sizePx.toFloat())
        val output = ByteArrayOutputStream()
        transcoder.transcode(TranscoderInput(svgInput), TranscoderOutput(output))
        return output.toByteArray()
    }
}
