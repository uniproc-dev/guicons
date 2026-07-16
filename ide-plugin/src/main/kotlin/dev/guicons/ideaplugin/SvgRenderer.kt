package dev.guicons.ideaplugin

import org.apache.batik.transcoder.TranscoderInput
import org.apache.batik.transcoder.TranscoderOutput
import org.apache.batik.transcoder.image.PNGTranscoder
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.FileInputStream

/** Rasterizes an SVG file to PNG bytes at a fixed square size, for
 * embedding in the doc popup - see [GuiconsDocumentationProvider]. */
object SvgRenderer {
    fun renderToPngBytes(svgFile: File, sizePx: Int): ByteArray {
        val transcoder = PNGTranscoder()
        transcoder.addTranscodingHint(PNGTranscoder.KEY_WIDTH, sizePx.toFloat())
        transcoder.addTranscodingHint(PNGTranscoder.KEY_HEIGHT, sizePx.toFloat())
        val output = ByteArrayOutputStream()
        FileInputStream(svgFile).use { input ->
            transcoder.transcode(TranscoderInput(input), TranscoderOutput(output))
        }
        return output.toByteArray()
    }
}
