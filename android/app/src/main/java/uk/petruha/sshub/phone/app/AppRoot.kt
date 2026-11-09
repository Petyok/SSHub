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
import androidx.compose.runtime.LaunchedEffect
import kotlinx.coroutines.launch
import androidx.compose.runtime.setValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.background
import androidx.compose.runtime.collectAsState
import androidx.navigation.NavBackStackEntry
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import uk.petruha.sshub.phone.ui.screens.identity.StateLostScreen
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
import uk.petruha.sshub.phone.ui.screens.trust.handoverRoute
import uk.petruha.sshub.phone.ui.screens.trust.trustRenewRoute
import uk.petruha.sshub.phone.ui.theme.LocalReducedMotion
import uk.petruha.sshub.phone.ui.theme.SshubTheme
import uk.petruha.sshub.phone.core.OpenedTrustExtend
import uk.petruha.sshub.phone.core.OpenedPhoneHandover
import uk.petruha.sshub.phone.core.OpenedTrustRenew
import uk.petruha.sshub.phone.core.OpenedTrustRequest
import uk.petruha.sshub.phone.core.RecoveryNoticeOpened
import uk.petruha.sshub.phone.core.ReshareOpened

/** Routes with no bottom bar: first run, lock, scan and the enroll steps 2 to 5 (spec 12, Nav column "none"). */
/** Routes that are meant to be shown while the vault is not Unlocked. */
private val LOCK_SAFE_ROUTES = setOf(Routes.Lock, Routes.LockRestore, Routes.Restore, Routes.FirstRun, Routes.StateLost)

/** D17: only Lock and About stay open while the block is set. Restore, lock-restore, first run and the stop screen are closed too (typing the 24 words into a build that was reported as untrusted is the exposure). */
private val MISMATCH_SAFE_ROUTES = setOf(Routes.Lock, Routes.About)

private fun hidesBar(route: String?): Boolean {
    val r = route?.substringBefore("/") ?: return true
    return r in setOf(
        Routes.FirstRun, Routes.Restore, Routes.Lock, Routes.LockRestore, Routes.Scan,
        Routes.Enroll2, Routes.Enroll3, Routes.Enroll5, Routes.EnrollBt, Routes.EnrollPaste, Routes.EnrollSound,
    )
}

private fun tabOf(route: String?): NavTab? = when (route?.substringBefore("/")) {
    Routes.Home, Routes.ChainHead -> NavTab.Ring
    Routes.Approvals, Routes.Red, Routes.Release, Routes.Refusal, Routes.TrustRequest, Routes.TrustSigned, Routes.TrustExtend, Routes.TrustRenew, Routes.Handover -> NavTab.Approvals
    Routes.Enroll1, Routes.Enroll4, Routes.EnrollDone -> NavTab.Enroll
    else -> NavTab.Security
}

/** Where a decoded frame is shown, across all groups. Null: the caller (chain head, enrollment) takes the result itself. */
class AppRouter(private val blocked: () -> Boolean = { false }) : OpenedRouter {
    /** D17: while the signer mismatch is set no scanned frame opens a flow. */
    override fun route(opened: Opened): String? = if (blocked()) null else routeFor(opened) ?: when (opened) {
        is OpenedTrustRequest -> trustRequestRoute(opened.view.token)
        is OpenedTrustExtend -> trustExtendRoute(opened.view.token)
        is OpenedTrustRenew -> trustRenewRoute(opened.view.token)
        is OpenedPhoneHandover -> handoverRoute(opened.view.token)
        is ReshareOpened -> reshareRoute(opened.token)
        is RecoveryNoticeOpened -> recoveryNoticeRoute(opened.token)
        else -> null
    }
}

@Composable
fun AppRoot(core: PhoneCoreApi, platform: Platform, settings: SettingsStore = AppSettings) {
    val theme by settings.theme.collectAsState()
    val mismatch by settings.signerMismatch.collectAsState()
    val verified by settings.backupVerified.collectAsState()
    val vault by platform.keys.state.collectAsState()
    val dark = when (theme) { ThemeChoice.Dark -> true; ThemeChoice.Light -> false; ThemeChoice.System -> isSystemInDarkTheme() }
    val reduced = platform.system.reducedMotion
    SshubTheme(dark = dark, reducedMotion = reduced) {
        CompositionLocalProvider(LocalPlatform provides platform) {
            val nav = rememberNavController()
            val env = remember(core, platform, nav) { AppEnv(core, platform, nav, settings) }
            val entry by nav.currentBackStackEntryAsState()
            val route = entry?.destination?.route
            val start = remember { if (vault == VaultState.NoKey) Routes.FirstRun else if (vault == VaultState.Unlocked) Routes.Home else lockRoute(LockReason.Started) }

            // One lock path: every source (screen off, the auto-lock timer, the Lock button, a restored back stack after process death)
            // ends as a vault that is not Unlocked, and the UI follows that state here. The platform and the vault do the locking.
            // The badge is read when the screen or the lock changes, never during composition (the core answers on its own thread).
            var pendingN by remember { mutableIntStateOf(0) }
            LaunchedEffect(route, vault) { pendingN = runCatching { core.pending(platform.clock.nowSeconds()).size }.getOrDefault(0) }
            val scope = rememberCoroutineScope()
            // Security M2: an unreadable saved state stops everything until the user read the stop screen.
            LaunchedEffect(vault, route) {
                if (route == null || vault != VaultState.Unlocked) return@LaunchedEffect
                if (core.stateLost() && route != Routes.StateLost) nav.navigate(Routes.StateLost) { popUpTo(nav.graph.id) { inclusive = true } }
            }
            // D17: with the signer mismatch set every flow except About and Lock stays closed, also after a restart.
            LaunchedEffect(mismatch, route) {
                // The stop screen is not in the safe list but must not be fought with: it ends first, then the block applies.
                if (route == null || !mismatch || core.stateLost()) return@LaunchedEffect
                if (route.substringBefore("/") !in MISMATCH_SAFE_ROUTES) nav.navigate(Routes.About) { popUpTo(nav.graph.id) { inclusive = true } }
            }
            LaunchedEffect(vault, route) {
                if (route == null) return@LaunchedEffect
                val base = route.substringBefore("/")
                if ((vault == VaultState.Locked || vault == VaultState.Invalidated) && base !in LOCK_SAFE_ROUTES) {
                    nav.navigate(lockRoute(LockReason.ScreenOff)) { popUpTo(nav.graph.id) { inclusive = true } }
                }
            }
            // The paper words are checked once, before the first approval: an approval screen opens the two-word check on top of itself (the back stack stays, so Back reaches the list).
            LaunchedEffect(vault, route, verified, mismatch) {
                if (route == null || verified || mismatch || vault != VaultState.Unlocked) return@LaunchedEffect
                if (route.substringBefore("/") in Routes.approvalBases) nav.navigate(Routes.BackupVerify)
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
                        composable(Routes.StateLost) {
                            StateLostScreen(
                                onContinue = { scope.launch { core.confirmStateLost(); nav.navigate(Routes.Home) { popUpTo(nav.graph.id) { inclusive = true } } } },
                                onLock = { core.lock(); platform.keys.lock() },
                            )
                        }
                        homeGraph(env, AppRouter { settings.signerMismatch.value }, settings.updateNeededAfter)
                        releaseGraph(env)
                        trustGraph(env)
                        recoveryGraph(env)
                        enrollGraph(env)
                    }
                }
                if (!hidesBar(route) && !mismatch) {
                    SshubNavBar(
                        tabOf(route), pending = pendingN, modifier = Modifier.fillMaxWidth(),
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
