package uk.petruha.sshub.phone.ui.screens.recovery

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import androidx.navigation.NavGraphBuilder
import androidx.navigation.NavType
import androidx.navigation.compose.composable
import androidx.navigation.navArgument
import kotlinx.coroutines.delay
import uk.petruha.sshub.phone.core.AppEnv
import uk.petruha.sshub.phone.core.Routes
import uk.petruha.sshub.phone.core.ScanPurpose
import uk.petruha.sshub.phone.core.scanRoute

/** Review of a reshare that `openFrame` returned as `ReshareOpened`. */
fun reshareRoute(token: Long) = Routes.Reshare + "/" + token

/** A recovery notice, from the recovery list or from `RecoveryNoticeOpened`. */
fun recoveryNoticeRoute(token: Long) = Routes.RecoveryNotice + "/" + token

private const val TOKEN = "token"
private val tokenArg = listOf(navArgument(TOKEN) { type = NavType.LongType })

fun NavGraphBuilder.recoveryGraph(env: AppEnv) {
    composable(Routes.Reshare) { ReshareRoute(env, null) }
    composable(Routes.Reshare + "/{$TOKEN}", arguments = tokenArg) { ReshareRoute(env, it.arguments?.getLong(TOKEN)) }
    composable(Routes.Recovery) { RecoveryRoute(env) }
    composable(Routes.RecoveryNotice + "/{$TOKEN}", arguments = tokenArg) { RecoveryNoticeRoute(env, it.arguments?.getLong(TOKEN) ?: -1L) }
    composable(Routes.Export) { ExportRoute(env) }
}

@Composable
private fun ReshareRoute(env: AppEnv, token: Long?) {
    val vm = viewModel<ReshareViewModel>(key = "reshare-$token", factory = viewModelFactory { initializer { ReshareViewModel(env.core, env.platform, token, env.settings) } })
    val s by vm.state.collectAsStateWithLifecycle()
    ReshareScreen(
        s,
        ReshareActions(
            onBack = { env.nav.popBackStack() }, onRotate = vm::rotate, onScan = { env.nav.navigate(scanRoute(ScanPurpose.Reshare)) },
            onAdoptPublish = vm::adoptPublish, onCopy = vm::copy,
            onExport = { env.nav.navigate(Routes.Export) }, onDone = { env.nav.popBackStack() },
        ),
    )
}

@Composable
private fun RecoveryRoute(env: AppEnv) {
    val vm = viewModel<RecoveryViewModel>(factory = viewModelFactory { initializer { RecoveryViewModel(env.core, env.platform) } })
    val s by vm.state.collectAsStateWithLifecycle()
    LaunchedEffect(Unit) { while (true) { delay(30_000); vm.tick() } }
    RecoveryScreen(
        s,
        RecoveryActions(
            onBack = { env.nav.popBackStack() }, onScan = { env.nav.navigate(scanRoute(ScanPurpose.Recovery)) },
            onOpenNotice = { env.nav.navigate(recoveryNoticeRoute(it)) },
        ),
    )
}

@Composable
private fun RecoveryNoticeRoute(env: AppEnv, token: Long) {
    val vm = viewModel<RecoveryNoticeViewModel>(key = "notice-$token", factory = viewModelFactory { initializer { RecoveryNoticeViewModel(env.core, env.platform, token) } })
    val s by vm.state.collectAsStateWithLifecycle()
    LaunchedEffect(Unit) { while (true) { delay(30_000); vm.tick() } }
    RecoveryNoticeScreen(
        s,
        NoticeActions(
            onBack = { env.nav.popBackStack() }, onAskCancel = vm::askCancel, onSign = vm::sign, onNotYet = vm::notYet,
            onLeave = { env.nav.popBackStack() }, onCopy = vm::copy, onDone = { env.nav.popBackStack() },
        ),
    )
}

@Composable
private fun ExportRoute(env: AppEnv) {
    val vm = viewModel<ExportOpsViewModel>(factory = viewModelFactory { initializer { ExportOpsViewModel(env.core, env.platform) } })
    val s by vm.state.collectAsStateWithLifecycle()
    ExportOpsScreen(s, ExportActions(onBack = { env.nav.popBackStack() }, onCopy = vm::copy, onDone = { env.nav.popBackStack() }))
}
