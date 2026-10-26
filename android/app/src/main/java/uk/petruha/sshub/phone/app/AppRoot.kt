package uk.petruha.sshub.phone.app

import androidx.compose.animation.AnimatedContentTransitionScope
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.background
import androidx.compose.runtime.collectAsState
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.navigation.NavBackStackEntry
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import uk.petruha.sshub.phone.core.AppEnv
import uk.petruha.sshub.phone.core.AppSettings
import uk.petruha.sshub.phone.core.LocalPlatform
import uk.petruha.sshub.phone.core.Opened
import uk.petruha.sshub.phone.core.OpenedRouter
import uk.petruha.sshub.phone.core.PhoneCoreApi
import uk.petruha.sshub.phone.core.Platform
import uk.petruha.sshub.phone.core.Routes
import uk.petruha.sshub.phone.core.SettingsStore
import uk.petruha.sshub.phone.core.ThemeChoice
import uk.petruha.sshub.phone.core.OpenedRed
import uk.petruha.sshub.phone.core.VaultState
import uk.petruha.sshub.phone.ui.components.NavTab
import uk.petruha.sshub.phone.ui.components.SshubNavBar
import uk.petruha.sshub.phone.ui.screens.enroll.enrollGraph
import uk.petruha.sshub.phone.ui.screens.home.homeGraph
import uk.petruha.sshub.phone.ui.screens.identity.identityGraph
import uk.petruha.sshub.phone.ui.screens.identity.lockRoute
import uk.petruha.sshub.phone.ui.screens.identity.LockReason
import uk.petruha.sshub.phone.ui.screens.recovery.recoveryGraph
import uk.petruha.sshub.phone.ui.screens.recovery.reshareRoute
import uk.petruha.sshub.phone.ui.screens.recovery.recoveryNoticeRoute
import uk.petruha.sshub.phone.ui.screens.release.releaseGraph
import uk.petruha.sshub.phone.ui.screens.release.routeFor
import uk.petruha.sshub.phone.ui.screens.trust.trustGraph
import uk.petruha.sshub.phone.ui.screens.trust.trustRequestRoute
import uk.petruha.sshub.phone.ui.screens.trust.trustExtendRoute
import uk.petruha.sshub.phone.ui.theme.LocalReducedMotion
import uk.petruha.sshub.phone.ui.theme.SshubTheme
import uk.petruha.sshub.phone.core.OpenedTrustExtend
import uk.petruha.sshub.phone.core.OpenedTrustRequest
import uk.petruha.sshub.phone.core.RecoveryNoticeOpened
import uk.petruha.sshub.phone.core.ReshareOpened

/** Routes with no bottom bar: first run, lock, scan and the enroll steps 2 to 5 (spec 12, Nav column "none"). */
private fun hidesBar(route: String?): Boolean {
    val r = route?.substringBefore("/") ?: return true
    return r in setOf(
        Routes.FirstRun, Routes.Restore, Routes.Lock, Routes.LockRestore, Routes.Scan,
        Routes.Enroll2, Routes.Enroll3, Routes.Enroll5, Routes.EnrollBt, Routes.EnrollPaste, Routes.EnrollSound,
    )
}

private fun tabOf(route: String?): NavTab? = when (route?.substringBefore("/")) {
    Routes.Home, Routes.ChainHead -> NavTab.Ring
    Routes.Approvals, Routes.Red, Routes.Release, Routes.Refusal, Routes.TrustRequest, Routes.TrustSigned, Routes.TrustExtend -> NavTab.Approvals
    Routes.Enroll1, Routes.Enroll4, Routes.EnrollDone -> NavTab.Enroll
    else -> NavTab.Security
}

/** Where a decoded frame is shown, across all groups. Null: the caller (chain head, enrollment) takes the result itself. */
class AppRouter : OpenedRouter {
    override fun route(opened: Opened): String? = routeFor(opened) ?: when (opened) {
        is OpenedTrustRequest -> trustRequestRoute(opened.view.token)
        is OpenedTrustExtend -> trustExtendRoute(opened.view.token)
        is ReshareOpened -> reshareRoute(opened.token)
        is RecoveryNoticeOpened -> recoveryNoticeRoute(opened.token)
        else -> null
    }
}

@Composable
fun AppRoot(core: PhoneCoreApi, platform: Platform, settings: SettingsStore = AppSettings) {
    val theme by settings.theme.collectAsState()
    val mismatch by settings.signerMismatch.collectAsState()
    val autoLock by settings.autoLockSeconds.collectAsState()
    val vault by platform.keys.state.collectAsState()
    val dark = when (theme) { ThemeChoice.Dark -> true; ThemeChoice.Light -> false; ThemeChoice.System -> isSystemInDarkTheme() }
    val reduced = platform.system.reducedMotion
    SshubTheme(dark = dark, reducedMotion = reduced) {
        CompositionLocalProvider(LocalPlatform provides platform) {
            val nav = rememberNavController()
            val env = remember(core, platform, nav) { AppEnv(core, platform, nav) }
            val entry by nav.currentBackStackEntryAsState()
            val route = entry?.destination?.route
            val start = remember { if (vault == VaultState.NoKey) Routes.FirstRun else if (vault == VaultState.Unlocked) Routes.Home else lockRoute(LockReason.Started) }

            // Lock when the app leaves the foreground for longer than the auto-lock time (spec 11), and at once on screen off.
            val owner = LocalLifecycleOwner.current
            DisposableEffect(owner, autoLock) {
                var stoppedAt = 0L
                val o = LifecycleEventObserver { _, e ->
                    if (e == Lifecycle.Event.ON_STOP) stoppedAt = platform.clock.nowSeconds()
                    if (e == Lifecycle.Event.ON_START && stoppedAt != 0L && platform.clock.nowSeconds() - stoppedAt >= autoLock && vault != VaultState.NoKey) {
                        core.lock(); platform.keys.lock()
                        nav.navigate(lockRoute(LockReason.Started)) { popUpTo(nav.graph.id) { inclusive = true } }
                    }
                }
                owner.lifecycle.addObserver(o); onDispose { owner.lifecycle.removeObserver(o) }
            }

            Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
                Box(Modifier.weight(1f)) {
                    val fade = if (reduced) tween<Float>(0) else tween(300)
                    val slidePx = with(LocalDensity.current) { 30.dp.roundToPx() }
                    NavHost(
                        nav, startDestination = start,
                        enterTransition = { if (reduced) fadeIn(tween(0)) else slideInHorizontally(tween(300)) { slidePx } + fadeIn(fade) },
                        exitTransition = { if (reduced) fadeOut(tween(0)) else fadeOut(tween(90)) },
                        popEnterTransition = { if (reduced) fadeIn(tween(0)) else slideInHorizontally(tween(300)) { -slidePx } + fadeIn(fade) },
                        popExitTransition = { if (reduced) fadeOut(tween(0)) else fadeOut(tween(90)) },
                    ) {
                        identityGraph(env, settings)
                        homeGraph(env, AppRouter())
                        releaseGraph(env)
                        trustGraph(env)
                        recoveryGraph(env)
                        enrollGraph(env)
                    }
                }
                if (!hidesBar(route) && !mismatch) {
                    SshubNavBar(
                        tabOf(route), pending = pendingCount(core, platform), modifier = Modifier.fillMaxWidth(),
                        onSelect = { tab ->
                            val dest = when (tab) { NavTab.Ring -> Routes.Home; NavTab.Approvals -> Routes.Approvals; NavTab.Enroll -> Routes.Enroll1; NavTab.Security -> Routes.Menu }
                            nav.navigate(dest) { popUpTo(Routes.Home); launchSingleTop = true }
                        },
                    )
                }
            }
        }
    }
}

private fun pendingCount(core: PhoneCoreApi, platform: Platform): Int =
    runCatching { core.pending(platform.clock.nowSeconds()).size }.getOrDefault(0)
