package uk.petruha.sshub.phone.ui.screens.identity

import android.content.ActivityNotFoundException
import android.content.Intent
import android.provider.Settings
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.NavGraphBuilder
import androidx.navigation.NavType
import androidx.navigation.compose.composable
import androidx.navigation.navArgument
import uk.petruha.sshub.phone.core.AppEnv
import uk.petruha.sshub.phone.core.AppSettings
import uk.petruha.sshub.phone.core.Routes
import uk.petruha.sshub.phone.core.ScanPurpose
import uk.petruha.sshub.phone.core.ScanResults
import uk.petruha.sshub.phone.core.SettingsStore
import uk.petruha.sshub.phone.core.scanRoute

/** `backup/first` right after the key was created (no way back), `backup/menu` from Security and About. */
fun backupRoute(firstRun: Boolean): String = Routes.Backup + "/" + if (firstRun) "first" else "menu"

/** `firstrun-restore` on a fresh phone, `firstrun-restore/over` over a stored key (after the ERASE gate). */
fun restoreRoute(replaces: Boolean): String = if (replaces) Routes.Restore + "/over" else Routes.Restore

fun lockRoute(reason: LockReason): String = Routes.Lock + "/" + reason.name.lowercase()

private fun <T : ViewModel> factory(make: () -> T): ViewModelProvider.Factory = object : ViewModelProvider.Factory {
    @Suppress("UNCHECKED_CAST")
    override fun <V : ViewModel> create(modelClass: Class<V>): V = make() as V
}

@Composable
private fun rememberOpenSecuritySettings(): () -> Unit {
    val ctx = LocalContext.current
    return remember(ctx) {
        {
            try {
                ctx.startActivity(Intent(Settings.ACTION_SECURITY_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            } catch (e: ActivityNotFoundException) {
                // no settings app on this device: the explanation on the screen is all there is
            }
        }
    }
}

/**
 * Identity screens: first run, restore, backup, lock, lock-restore, chain-head, the Security hub and About.
 * `settings` is shared with the app shell (theme, auto-lock, the D17 block); the in-memory default is fine until the shell persists it.
 */
fun NavGraphBuilder.identityGraph(env: AppEnv, settings: SettingsStore = AppSettings) {
    val nav = env.nav
    fun clearTo(route: String) = nav.navigate(route) { popUpTo(nav.graph.id) { inclusive = true } }

    composable(Routes.FirstRun) {
        val vm: FirstRunViewModel = viewModel(factory = factory { FirstRunViewModel(env.core, env.platform) })
        val state by vm.state.collectAsStateWithLifecycle()
        FirstRunScreen(
            state,
            FirstRunActions(
                onCreate = { vm.create { nav.navigate(backupRoute(true)) { popUpTo(Routes.FirstRun) { inclusive = true } } } },
                onRestore = { nav.navigate(restoreRoute(false)) },
                onOpenSecuritySettings = rememberOpenSecuritySettings(),
            ),
        )
    }

    composable(Routes.Restore) { RestoreDestination(env, replaces = false) }
    composable(Routes.Restore + "/over") { RestoreDestination(env, replaces = true) }

    composable(Routes.Backup) { BackupDestination(env, settings, firstRun = false) }
    composable(Routes.Backup + "/{origin}", arguments = listOf(navArgument("origin") { type = NavType.StringType })) { entry ->
        BackupDestination(env, settings, firstRun = entry.arguments?.getString("origin") == "first")
    }

    composable(Routes.Lock) { LockDestination(env, LockReason.Started) }
    composable(Routes.Lock + "/{reason}", arguments = listOf(navArgument("reason") { type = NavType.StringType })) { entry ->
        val reason = LockReason.entries.firstOrNull { it.name.lowercase() == entry.arguments?.getString("reason") } ?: LockReason.Started
        LockDestination(env, reason)
    }

    composable(Routes.LockRestore) {
        val vm: ReplaceKeyViewModel = viewModel(factory = factory { ReplaceKeyViewModel() })
        val state by vm.state.collectAsStateWithLifecycle()
        ReplaceKeyScreen(state, ReplaceKeyActions(onTyped = vm::onTyped, onCancel = { nav.popBackStack() }, onErase = { nav.navigate(restoreRoute(true)) }))
    }

    composable(Routes.ChainHead) {
        val vm: ChainHeadViewModel = viewModel(factory = factory { ChainHeadViewModel(env.core, env.core, env.platform) })
        val state by vm.state.collectAsStateWithLifecycle()
        val latest by ScanResults.latest.collectAsStateWithLifecycle()
        LaunchedEffect(latest) {
            if (latest?.purpose == ScanPurpose.Chain) ScanResults.take(ScanPurpose.Chain)?.let { vm.onOpened(it.opened) }
        }
        TakeChainScreen(
            state,
            ChainHeadActions(
                onScan = { nav.navigate(scanRoute(ScanPurpose.Chain)) },
                onOpenPaste = vm::onOpenPaste, onClosePaste = vm::onClosePaste, onPasteText = vm::onPasteText, onCheckPaste = vm::checkPaste,
                onAdopt = vm::adopt, onTypedId = vm::onTypedId, onNeverSigned = vm::neverSigned, onTryAnother = vm::tryAnother,
                onHome = { clearTo(Routes.Home) }, onBack = { clearTo(Routes.Home) },
            ),
        )
    }

    composable(Routes.Menu) {
        val vm: SecurityMenuViewModel = viewModel(factory = factory { SecurityMenuViewModel(env.core, env.platform, settings) })
        val state by vm.state.collectAsStateWithLifecycle()
        SecurityMenuScreen(
            state,
            SecurityMenuActions(
                onTrustList = { nav.navigate(Routes.TrustList) }, onCodes = { nav.navigate(Routes.Codes) }, onReshare = { nav.navigate(Routes.Reshare) },
                onRecovery = { nav.navigate(Routes.Recovery) }, onBackup = { nav.navigate(backupRoute(false)) }, onExport = { nav.navigate(Routes.Export) },
                onAbout = { nav.navigate(Routes.About) },
                onLock = { vm.lock(); clearTo(lockRoute(LockReason.Manual)) },
                onTheme = vm::setTheme, onAutoLock = vm::setAutoLock,
            ),
        )
    }

    composable(Routes.About) {
        val vm: AboutViewModel = viewModel(factory = factory { AboutViewModel(env.core, env.platform, settings) })
        val state by vm.state.collectAsStateWithLifecycle()
        AboutSecurityScreen(
            state,
            AboutActions(
                onToggleCompare = vm::toggleCompare, onReference = vm::onReference, onToggleGroup = vm::toggleGroup,
                onBackup = { nav.navigate(backupRoute(false)) }, onToggleLicences = vm::toggleLicences, onBack = { nav.popBackStack() },
            ),
        )
    }
}

@Composable
private fun RestoreDestination(env: AppEnv, replaces: Boolean) {
    val vm: RestoreViewModel = viewModel(factory = factory { RestoreViewModel(env.core, env.platform, replaces) })
    val state by vm.state.collectAsStateWithLifecycle()
    val openSettings = rememberOpenSecuritySettings()
    RestoreScreen(
        state,
        RestoreActions(
            onWord = vm::onWord, onPaste = vm::onPaste, onFill = vm::onFill, onSiteDataCleared = vm::onSiteDataCleared,
            onRestore = { vm.restore { env.nav.navigate(Routes.ChainHead) { popUpTo(env.nav.graph.id) { inclusive = true } } } },
            onBack = { env.nav.popBackStack() }, onOpenSecuritySettings = openSettings,
        ),
    )
}

@Composable
private fun BackupDestination(env: AppEnv, settings: SettingsStore, firstRun: Boolean) {
    val vm: BackupViewModel = viewModel(factory = factory { BackupViewModel(env.core, env.platform, settings, firstRun) })
    val state by vm.state.collectAsStateWithLifecycle()
    BackupWordsScreen(
        state,
        BackupActions(
            onPaperChecked = vm::onPaperChecked, onAuthenticate = vm::authenticate, onBack = { env.nav.popBackStack() },
            onDone = {
                vm.done {
                    if (firstRun) env.nav.navigate(Routes.Home) { popUpTo(env.nav.graph.id) { inclusive = true } } else env.nav.popBackStack()
                }
            },
        ),
    )
}

@Composable
private fun LockDestination(env: AppEnv, reason: LockReason) {
    val vm: LockViewModel = viewModel(factory = factory { LockViewModel(env.core, env.platform, reason) })
    val state by vm.state.collectAsStateWithLifecycle()
    val unlocked by vm.unlocked.collectAsStateWithLifecycle()
    LaunchedEffect(unlocked) {
        if (unlocked) env.nav.navigate(Routes.Home) { popUpTo(env.nav.graph.id) { inclusive = true } }
    }
    LockScreen(state, LockActions(onUnlock = vm::unlock, onRestore = { env.nav.navigate(Routes.LockRestore) }))
}
