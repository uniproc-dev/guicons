package dev.guicons.ideaplugin

import com.intellij.ui.JBColor
import java.awt.Color
import java.awt.Graphics2D
import java.awt.RenderingHints
import java.awt.image.BufferedImage
import java.io.ByteArrayOutputStream
import javax.imageio.ImageIO

/**
 * Shared between [IconBrowserPopup] (a live-painted [javax.swing.JPanel]
 * card, redrawn on every selection) and [IconDocRenderer] (a static PNG
 * baked once and inlined into the Quick Doc popup's HTML, which has no
 * equivalent to a custom `paintComponent`) - same problem either way: an
 * icon whose SVG has no fill set renders solid black, invisible against
 * a fixed-color background, so the card has to be colored to contrast
 * with the icon's *own* pixels instead.
 */
object IconPreviewCard {
    /** Average, alpha-weighted luminance of `image`'s opaque-ish pixels
     * (sparsely sampled - a preview icon is small, a full scan is wasted
     * work), picking light-on-dark-icon or dark-on-light-icon so the icon
     * is never the same shade as its own background. */
    fun contrastingCardColor(image: BufferedImage?): Color {
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

    /** Bakes `image` onto a fixed-size rounded-rect card (colored via
     * [contrastingCardColor]) and re-encodes the result as PNG bytes -
     * for [IconDocRenderer]'s HTML `<img>`, which can only embed a static
     * image, not repaint a live Swing component the way
     * [IconBrowserPopup]'s card does. */
    fun renderCardPng(image: BufferedImage, cardSize: Int, arc: Int): ByteArray {
        val card = BufferedImage(cardSize, cardSize, BufferedImage.TYPE_INT_ARGB)
        val g2: Graphics2D = card.createGraphics()
        try {
            g2.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON)
            g2.color = contrastingCardColor(image)
            g2.fillRoundRect(0, 0, cardSize, cardSize, arc, arc)
            g2.drawImage(image, (cardSize - image.width) / 2, (cardSize - image.height) / 2, null)
        } finally {
            g2.dispose()
        }
        val out = ByteArrayOutputStream()
        ImageIO.write(card, "png", out)
        return out.toByteArray()
    }
}
