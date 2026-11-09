package uk.petruha.sshub.phone.ui.screens.trust

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.background
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import androidx.navigation.NavGraphBuilder
import androidx.navigation.NavType
import androidx.navigation.compose.composable
import androidx.navigation.navArgument
import uk.petruha.sshub.phone.core.AppEnv
import uk.petruha.sshub.phone.core.Routes
import uk.petruha.sshub.phone.core.ScanPurpose
import uk.petruha.sshub.phone.core.scanRoute
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.screens.release.ConsumeScannedDeviceId

/** Argument routes of this group: `Routes.TrustRequest + "/{token}"` and `Routes.TrustExtend + "/{token}"`; the token is the one in `Opened.TrustRequest`/`TrustExtend`. */
fun trustRequestRoute(token: Long) = "${Routes.TrustRequest}/$token"
fun trustExtendRoute(token: Long) = "${Routes.TrustExtend}/$token"
fun trustRenewRoute(token: Long) = "${Routes.TrustRenew}/$token"
fun handoverRoute(token: Long) = "${Routes.Handover}/$token"

fun NavGraphBuilder.trustGraph(env: AppEnv) {
    val handoffs = trustHandoffs(env)
    val nav = env.nav
    fun back() { nav.popBackStack() }
    fun backTo(route: String) { if (!nav.popBackStack(route, false)) nav.navigate(route) }
    val longArg = listOf(navArgument("token") { type = NavType.LongType })

    composable(Routes.TrustRequest + "/{token}", arguments = longArg) { entry ->
        val token = entry.arguments?.getLong("token") ?: 0L
        val vm: TrustRequestViewModel = viewModel(factory = viewModelFactory { initializer { TrustRequestViewModel(env.core, env.platform, token, handoffs) } })
        val s by vm.state.collectAsStateWithLifecycle()
        val st = s
        ConsumeScannedDeviceId(vm::typed)
        if (st == null) TrustMissing("Trust request", ::back)
        else TrustRequestScreen(
            st,
            TrustRequestActions(
                onTyped = vm::typed,
                onApprove = { vm.approve { nav.navigate(Routes.TrustSigned) { popUpTo(Routes.TrustRequest + "/{token}") { inclusive = true } } } },
                onDecline = vm::decline, onBack = { backTo(Routes.Approvals) },
                onScanId = { nav.navigate(scanRoute(ScanPurpose.DeviceId)) },
            ),
        )
    }
    composable(Routes.TrustExtend + "/{token}", arguments = longArg) { entry ->
        val token = entry.arguments?.getLong("token") ?: 0L
        val vm: TrustExtendViewModel = viewModel(factory = viewModelFactory { initializer { TrustExtendViewModel(env.core, env.platform, token) } })
        val s by vm.state.collectAsStateWithLifecycle()
        val st = s
        if (st == null) TrustMissing("Budget extension", ::back)
        else TrustExtendScreen(
            st,
            TrustExtendActions(
                onApprove = vm::approve, onDecline = vm::decline, onCopy = vm::copy,
                onDone = { backTo(Routes.Approvals) }, onBack = { backTo(Routes.Approvals) },
            ),
        )
    }
    composable(Routes.TrustRenew + "/{token}", arguments = longArg) { entry ->
        val token = entry.arguments?.getLong("token") ?: 0L
        val vm: TrustRenewViewModel = viewModel(factory = viewModelFactory { initializer { TrustRenewViewModel(env.core, env.platform, token) } })
        val s by vm.state.collectAsStateWithLifecycle()
        val st = s
        if (st == null) TrustMissing("Renew a trust session", ::back)
        else TrustRenewScreen(
            st,
            TrustRenewActions(
                onApprove = vm::approve, onDecline = vm::decline, onCopy = vm::copy,
                onDone = { backTo(Routes.Approvals) }, onBack = { backTo(Routes.Approvals) },
            ),
        )
    }
    composable(Routes.Handover + "/{token}", arguments = longArg) { entry ->
        val token = entry.arguments?.getLong("token") ?: 0L
        val vm: HandoverViewModel = viewModel(factory = viewModelFactory { initializer { HandoverViewModel(env.core, env.platform, token) } })
        val s by vm.state.collectAsStateWithLifecycle()
        val st = s
        if (st == null) TrustMissing("Replace this phone", ::back)
        else HandoverScreen(
            st,
            HandoverActions(
                onWordsMatch = vm::onWordsMatch, onApprove = vm::approve, onDecline = vm::decline, onCopy = vm::copy,
                onDone = { backTo(Routes.Approvals) }, onBack = { backTo(Routes.Approvals) },
            ),
        )
    }
    composable(Routes.TrustSigned) {
        val vm: TrustHandoffViewModel = viewModel(factory = viewModelFactory { initializer { TrustHandoffViewModel(env.platform, handoffs) } })
        val s by vm.state.collectAsStateWithLifecycle()
        TrustSignedScreen(s, TrustHandoffActions(onCopy = vm::copy, onDone = { backTo(Routes.Approvals) }, onBack = { backTo(Routes.Approvals) }))
    }
    composable(Routes.TrustList) {
        val vm: TrustListViewModel = viewModel(factory = viewModelFactory { initializer { TrustListViewModel(env.core, env.platform, handoffs) } })
        val s by vm.state.collectAsStateWithLifecycle()
        TrustListScreen(
            s,
            TrustListActions(
                onToggle = vm::toggle, onAskRevoke = vm::askRevoke, onKeep = vm::keep,
                onRevoke = { vm.revoke { nav.navigate(Routes.TrustRevoked) } },
                onScan = { nav.navigate(scanRoute(ScanPurpose.Request)) }, onBack = ::back,
            ),
        )
    }
    composable(Routes.TrustRevoked) {
        val vm: TrustHandoffViewModel = viewModel(factory = viewModelFactory { initializer { TrustHandoffViewModel(env.platform, handoffs) } })
        val s by vm.state.collectAsStateWithLifecycle()
        TrustRevokedScreen(s, TrustHandoffActions(onCopy = vm::copy, onDone = { backTo(Routes.TrustList) }, onBack = { backTo(Routes.TrustList) }))
    }
    composable(Routes.Codes) {
        val vm: ShortCodesViewModel = viewModel(factory = viewModelFactory { initializer { ShortCodesViewModel(env.core, env.platform) } })
        val s by vm.state.collectAsStateWithLifecycle()
        ShortCodesScreen(s, ShortCodesActions(onAskDead = vm::askDead, onKeep = vm::keep, onDeclareDead = vm::declareDead, onBack = ::back))
    }
    composable(Routes.CodeIssued) {
        val vm: CodeIssuedViewModel = viewModel(factory = viewModelFactory { initializer { CodeIssuedViewModel(env.core, env.platform) } })
        val s by vm.state.collectAsStateWithLifecycle()
        if (!s.loaded) Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background))
        else CodeIssuedScreen(s, CodeIssuedActions(onDone = { backTo(Routes.Codes) }, onBack = { backTo(Routes.Codes) }))
    }
}

/** The core no longer holds this request (lapsed, or the app was restarted). */
@Composable
private fun TrustMissing(title: String, onBack: () -> Unit) {
    ScreenFrame(title, onBack) { Note("This request is no longer available. Scan it again from the terminal.") }
}
