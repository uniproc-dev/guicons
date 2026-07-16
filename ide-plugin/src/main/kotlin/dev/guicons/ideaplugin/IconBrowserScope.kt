package dev.guicons.ideaplugin

import com.intellij.openapi.components.Service
import com.intellij.openapi.project.Project
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob

/**
 * Project-scoped `CoroutineScope` for the icon browser's `suspend`-fun
 * FFI calls (`cachedIconifyCollectionNames`/`searchIconifyIcons`/etc. -
 * see `guicons-ffi`'s `async_runtime = "tokio"` functions) - the standard
 * IntelliJ Platform idiom for getting a scope tied to project lifetime
 * without managing cancellation by hand (the platform injects the
 * constructor's `CoroutineScope` parameter and cancels it when the
 * project closes).
 */
@Service(Service.Level.PROJECT)
class IconBrowserScope(val coroutineScope: CoroutineScope) {
    companion object {
        /// A plain child of the platform-injected scope would propagate
        /// an uncaught exception in *any* `launch` (say, a bug in one
        /// popup tab) up through the shared parent `Job`, cancelling it -
        /// and with it every other tab/popup using this same
        /// project-level scope, permanently, until the project is
        /// reopened (hit exactly this: a crash in the Manifest tab
        /// silently broke the Iconify tab's previews too). A
        /// `SupervisorJob` keeps a failure local to whichever coroutine
        /// threw, while still getting cancelled itself when the
        /// platform's own scope closes (it's parented to that scope's Job).
        fun of(project: Project): CoroutineScope {
            val parent = project.getService(IconBrowserScope::class.java).coroutineScope
            return CoroutineScope(parent.coroutineContext + SupervisorJob(parent.coroutineContext[Job]))
        }
    }
}
