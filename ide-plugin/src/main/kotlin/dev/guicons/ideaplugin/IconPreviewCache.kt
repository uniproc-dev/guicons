package dev.guicons.ideaplugin

import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import uniffi.guicons_ffi.fetchIconifyIconPreview
import kotlin.time.Duration.Companion.minutes
import kotlin.time.TimeSource

/**
 * In-memory, TTL-expiring cache of iconify preview SVG bytes, fronting
 * `fetchIconifyIconPreview`'s network fetch. Lives here rather than on
 * the Rust side of the FFI boundary because this - the icon browser's
 * grid/single-selection preview - is the only caller that function has;
 * a stateless FFI function with the one caller that needs caching owning
 * the cache is simpler than threading state through a boundary for no
 * second consumer.
 *
 * A single process-wide object, not project-scoped - an iconify id
 * resolves to the same bytes regardless of which project asked. Entries
 * expire after [TTL] rather than living for the rest of the IDE process:
 * browsing/searching the Iconify tab can touch far more icons than a user
 * ever keeps in their manifest, and a long-running session shouldn't
 * accumulate every SVG ever scrolled past.
 */
object IconPreviewCache {
    private val TTL = 15.minutes
    private val mutex = Mutex()
    private val entries = HashMap<String, Entry>()

    private class Entry(val bytes: ByteArray, val fetchedAt: TimeSource.Monotonic.ValueTimeMark)

    suspend fun get(iconifyId: String): ByteArray? {
        val cached = mutex.withLock {
            val entry = entries[iconifyId]
            when {
                entry == null -> null
                entry.fetchedAt.elapsedNow() < TTL -> entry.bytes
                else -> {
                    entries.remove(iconifyId)
                    null
                }
            }
        }
        if (cached != null) return cached

        val bytes = fetchIconifyIconPreview(iconifyId) ?: return null
        mutex.withLock { entries[iconifyId] = Entry(bytes, TimeSource.Monotonic.markNow()) }
        return bytes
    }
}
