package uk.petruha.sshub.phone.ui.screens.release

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.NavBackStackEntry
import androidx.navigation.NavGraphBuilder
import androidx.navigation.NavType
import androidx.navigation.compose.composable
import androidx.navigation.navArgument
import uk.petruha.sshub.phone.core.AppEnv
import uk.petruha.sshub.phone.core.Opened
import uk.petruha.sshub.phone.core.OpenedRed
import uk.petruha.sshub.phone.core.OpenedRefused
import uk.petruha.sshub.phone.core.OpenedRelease
import uk.petruha.sshub.phone.core.Routes
import uk.petruha.sshub.phone.core.ScanPurpose
import uk.petruha.sshub.phone.core.scanRoute

/** Argument routes of this group, `Routes.X + "/{arg}"` (core/Env.kt owns the base ids). */
object ReleaseRoutes {
    const val RED = Routes.Red + "/{token}"
    const val RELEASE = Routes.Release + "/{token}"
    const val REFUSAL = Routes.Refusal + "/{id}"
    fun red(token: Long) = Routes.Red + "/" + token
    fun release(token: Long) = Routes.Release + "/" + token
    fun refusal(id: Long) = Routes.Refusal + "/" + id
}

/** A refusal is a value without a token, so the shell parks it here and navigates by the id it gets back. Process memory only. */
object RefusalStore {
    private val items = LinkedHashMap<Long, OpenedRefused>()
    private var next = 1L

    @Synchronized fun put(r: OpenedRefused): Long {
        val id = next++
        items[id] = r
        if (items.size > 16) items.remove(items.keys.first())
        return id
    }
    @Synchronized fun get(id: Long): OpenedRefused? = items[id]
}

/** Where `openFrame` result `o` is shown, or null when it belongs to another group. Called by Scan and Approvals. */
fun routeFor(o: Opened): String? = when (o) {
    is OpenedRed -> ReleaseRoutes.red(o.view.token)
    is OpenedRelease -> ReleaseRoutes.release(o.view.token)
    is OpenedRefused -> ReleaseRoutes.refusal(RefusalStore.put(o))
    else -> null
}

private fun <T : ViewModel> factory(make: () -> T) = object : ViewModelProvider.Factory {
    @Suppress("UNCHECKED_CAST")
    override fun <V : ViewModel> create(modelClass: Class<V>): V = make() as V
}

private fun NavBackStackEntry.long(name: String): Long = arguments?.getLong(name) ?: 0L

fun NavGraphBuilder.releaseGraph(env: AppEnv) {
    composable(ReleaseRoutes.RED, arguments = listOf(navArgument("token") { type = NavType.LongType })) { entry ->
        val token = entry.long("token")
        val vm: RedViewModel = viewModel(key = "red-$token", factory = factory { RedViewModel(env.core, env.platform, token, env.settings) })
        val state by vm.state.collectAsStateWithLifecycle()
        val s = state
        ConsumeScannedDeviceId(vm::typed)
        if (s == null) LaunchedEffect(Unit) { env.nav.popBackStack() }
        else RedScreen(
            s, RedActions(
                onBack = { env.nav.popBackStack() }, onApprove = vm::approve, onDecline = vm::decline,
                onToggleHash = vm::toggleHash, onCopy = vm::copy, onTyped = vm::typed,
                onScanId = { env.nav.navigate(scanRoute(ScanPurpose.DeviceId)) },
            ),
        )
    }
    composable(ReleaseRoutes.RELEASE, arguments = listOf(navArgument("token") { type = NavType.LongType })) { entry ->
        val token = entry.long("token")
        val vm: ReleaseViewModel = viewModel(key = "release-$token", factory = factory { ReleaseViewModel(env.core, env.platform, token) })
        val state by vm.state.collectAsStateWithLifecycle()
        val s = state
        ConsumeScannedDeviceId(vm::typed)
        if (s == null) LaunchedEffect(Unit) { env.nav.popBackStack() }
        else ShareReleaseScreen(
            s, ReleaseActions(
                onBack = { env.nav.popBackStack() }, onApprove = vm::approve, onDecline = vm::decline, onTyped = vm::typed,
                onChannel = vm::channel, onSound = vm::sound, onShortCode = vm::shortCode, onByHand = vm::byHand, onCopy = vm::copy,
                onAskAgain = { env.nav.navigate(scanRoute(ScanPurpose.Request)) },
                onScanId = { env.nav.navigate(scanRoute(ScanPurpose.DeviceId)) },
            ),
        )
    }
    composable(ReleaseRoutes.REFUSAL, arguments = listOf(navArgument("id") { type = NavType.LongType })) { entry ->
        val refused = remember { RefusalStore.get(entry.long("id")) }
        if (refused == null) LaunchedEffect(Unit) { env.nav.popBackStack() }
        else RefusalScreen(refused, RefusalActions(onBack = { env.nav.popBackStack() }, onScanAgain = { env.nav.navigate(scanRoute(ScanPurpose.Request)) }))
    }
}
