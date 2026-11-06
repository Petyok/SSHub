package uk.petruha.sshub.phone.ui.screens.enroll

import android.os.Build
import androidx.activity.ComponentActivity
import androidx.activity.compose.LocalActivity
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import androidx.navigation.NavGraphBuilder
import androidx.navigation.compose.composable
import uk.petruha.sshub.phone.core.AppEnv
import uk.petruha.sshub.phone.core.Routes

/** The nine enrollment destinations. One view model, scoped to the activity, carries the run across them. */
fun NavGraphBuilder.enrollGraph(env: AppEnv) {
    composable(Routes.Enroll1) {
        val vm = rememberEnrollViewModel(env)
        val ui by vm.ui.collectAsStateWithLifecycle()
        LaunchedEffect(Unit) { vm.restart() }
        EnrollScanScreen(
            ui.toScan(),
            EnrollScanActions(
                onBack = vm::back, onName = vm::setName, onChannel = vm::setChannel, onScan = vm::startScan, onCancelScan = vm::cancelScan,
                onFindTerminal = { vm.go(Routes.EnrollBt) }, onPasteText = vm::startPaste,
                receive = EnrollReceiveActions(vm::onLuma, vm::requestCamera, vm::openCameraSettings),
            ),
        )
    }
    composable(Routes.Enroll2) {
        val vm = rememberEnrollViewModel(env)
        val ui by vm.ui.collectAsStateWithLifecycle()
        EnrollShowScreen(ui.toShow(), EnrollShowActions(onBack = vm::back, onNext = { vm.go(Routes.Enroll3) }, send = vm.sendActions(2)))
    }
    composable(Routes.Enroll3) {
        val vm = rememberEnrollViewModel(env)
        val ui by vm.ui.collectAsStateWithLifecycle()
        EnrollKeysScreen(
            ui.toKeys(),
            EnrollKeysActions(
                onBack = vm::back, onCompare = { vm.go(Routes.Enroll4) }, onStartAgain = vm::startAgain,
                receive = EnrollReceiveActions(vm::onLuma, vm::requestCamera, vm::openCameraSettings),
            ),
        )
    }
    composable(Routes.Enroll4) {
        val vm = rememberEnrollViewModel(env)
        val ui by vm.ui.collectAsStateWithLifecycle()
        DisposableEffect(Unit) { onDispose { vm.soundStop() } }
        EnrollCompareScreen(
            ui.toCompare(),
            EnrollCompareActions(
                onBack = vm::back, onMatch = { vm.decideSas(true) }, onDiffer = { vm.decideSas(false) },
                onContinue = vm::continueToConfirm, onStartAgain = vm::startAgain, send = vm.sendActions(4),
            ),
        )
    }
    composable(Routes.Enroll5) {
        val vm = rememberEnrollViewModel(env)
        val ui by vm.ui.collectAsStateWithLifecycle()
        DisposableEffect(Unit) { onDispose { vm.soundStop() } }
        EnrollConfirmScreen(
            ui.toConfirm(),
            EnrollConfirmActions(
                onBack = vm::back, onFinish = { vm.go(Routes.EnrollDone) }, onStartAgain = vm::startAgain, send = vm.sendActions(5),
                receive = EnrollReceiveActions(vm::onLuma, vm::requestCamera, vm::openCameraSettings),
            ),
        )
    }
    composable(Routes.EnrollDone) {
        val vm = rememberEnrollViewModel(env)
        val ui by vm.ui.collectAsStateWithLifecycle()
        ui.toEnrolled()?.let { EnrolledScreen(it, EnrolledActions(onDone = vm::finish)) }
    }
    composable(Routes.EnrollBt) {
        val vm = rememberEnrollViewModel(env)
        val ui by vm.ui.collectAsStateWithLifecycle()
        LaunchedEffect(Unit) { vm.btStart() }
        DisposableEffect(Unit) { onDispose { vm.btStop() } }
        EnrollBtScreen(
            ui.toBt(),
            EnrollBtActions(
                onBack = vm::btCancel, onTag = vm::btSetTag, onSelect = vm::btSelect, onPair = vm::btPair,
                onSearchAgain = vm::btStart, onAllow = vm::btAllow, onOtherChannel = vm::back,
            ),
        )
    }
    composable(Routes.EnrollPaste) {
        val vm = rememberEnrollViewModel(env)
        val ui by vm.ui.collectAsStateWithLifecycle()
        EnrollPasteScreen(
            ui.toPaste(),
            EnrollPasteActions(
                onBack = vm::pasteBack, onText = vm::setPasteText, onPaste = vm::pasteFromClipboard, onClear = vm::pasteClear,
                onUse = vm::pasteUse, onNext = vm::pasteNext, send = vm.sendActions(ui.flow.pasteN),
            ),
        )
    }
    composable(Routes.EnrollSound) {
        val vm = rememberEnrollViewModel(env)
        val ui by vm.ui.collectAsStateWithLifecycle()
        DisposableEffect(Unit) { onDispose { vm.soundStop() } }
        LaunchedEffect(Unit) { vm.soundOpened() }
        EnrollSoundScreen(
            ui.toSound(),
            EnrollSoundActions(onBack = vm::back, onContinue = { vm.soundStop(); vm.go(Routes.Enroll3) }, send = vm.sendActions(ui.flow.soundStep)),
        )
    }
}

private fun EnrollViewModel.sendActions(step: Int) = EnrollSendActions(
    onToggleText = { toggleText(step) },
    onCopy = { copyFor(step) },
    onProfile = this::soundProfile,
    onPlay = { soundPlay(step) },
    onStop = this::soundStop,
)

@Composable
private fun rememberEnrollViewModel(env: AppEnv): EnrollViewModel {
    val owner = LocalActivity.current as ComponentActivity
    return viewModel<EnrollViewModel>(
        viewModelStoreOwner = owner,
        factory = viewModelFactory {
            initializer { EnrollViewModel(env.core, env.platform, env.nav, Build.MODEL.lowercase().replace(' ', '-')) }
        },
    )
}
