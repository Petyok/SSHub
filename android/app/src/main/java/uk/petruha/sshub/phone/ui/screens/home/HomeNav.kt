package uk.petruha.sshub.phone.ui.screens.home

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import androidx.navigation.NavGraphBuilder
import androidx.navigation.NavType
import androidx.navigation.compose.composable
import androidx.navigation.navArgument
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import uk.petruha.sshub.phone.core.AppEnv
import uk.petruha.sshub.phone.core.OpenedRouter
import uk.petruha.sshub.phone.core.PendingItem
import uk.petruha.sshub.phone.core.Routes
import uk.petruha.sshub.phone.core.ScanPurpose
import uk.petruha.sshub.phone.core.scanRoute

/**
 * Routes: `home`, `approvals`, `scan/{purpose}`. `router` turns a scanned `Opened` into a route (null: pop back and let the caller read
 * `ScanResults`). The shell provides `LocalPlatform` (the scan screen draws `platform.camera.Preview`).
 */
fun NavGraphBuilder.homeGraph(
    env: AppEnv,
    router: OpenedRouter = OpenedRouter { null },
    updateNeeded: StateFlow<String?> = MutableStateFlow(null),
) {
    composable(Routes.Home) {
        val vm: HomeViewModel = viewModel(factory = viewModelFactory { initializer { HomeViewModel(env.core, env.platform, updateNeeded) } })
        val state by vm.state.collectAsState()
        LaunchedEffect(Unit) { vm.refresh() }
        val shown = state
        val actions = remember(env) {
            HomeActions(
                onOpenPending = { env.nav.navigate(it.route) },
                onOpenApprovals = { env.nav.navigate(Routes.Approvals) },
                onEnroll = { env.nav.navigate(Routes.Enroll1) },
                onRestore = { env.nav.navigate(Routes.LockRestore) },
                onTakeChain = { env.nav.navigate(Routes.ChainHead) },
                onUpdateShares = { env.nav.navigate(Routes.Reshare) },
            )
        }
        if (shown != null) RingHomeScreen(shown, actions)
    }
    composable(Routes.Approvals) {
        val vm: ApprovalsViewModel = viewModel(factory = viewModelFactory { initializer { ApprovalsViewModel(env.core, env.platform) } })
        val state by vm.state.collectAsState()
        LaunchedEffect(Unit) { vm.refresh() }
        val actions = remember(env, vm) {
            ApprovalsActions(
                onSelect = vm::select,
                onOpen = { item: PendingItem -> env.nav.navigate(item.route) },
                onScan = { env.nav.navigate(scanRoute(ScanPurpose.Request)) },
            )
        }
        ApprovalsScreen(state, actions)
    }
    val scanPattern = Routes.Scan + "/{purpose}"
    composable(scanPattern, arguments = listOf(navArgument("purpose") { type = NavType.StringType })) { entry ->
        val purpose = ScanPurpose.fromArg(entry.arguments?.getString("purpose"))
        val vm: ScanViewModel = viewModel(factory = viewModelFactory { initializer { ScanViewModel(env.core, env.platform, purpose) } })
        val state by vm.state.collectAsState()
        val actions = remember(env, vm) {
            ScanActions(
                onBack = { env.nav.popBackStack() },
                onGrant = vm::grant,
                onOpenSettings = vm::openSettings,
                onTogglePaste = vm::togglePaste,
                onPasteText = vm::pasteText,
                onPasteFromClipboard = vm::pasteFromClipboard,
                onUsePasted = vm::usePasted,
                onContinue = {
                    val opened = vm.finish()
                    val route = opened?.let { router.route(it) }
                    if (route != null) env.nav.navigate(route) { popUpTo(scanPattern) { inclusive = true } }
                    else env.nav.popBackStack()
                },
                onLuma = vm::onLuma,
            )
        }
        ScanScreen(state, actions)
    }
}
