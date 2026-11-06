package uk.petruha.sshub.phone.enroll

import androidx.compose.runtime.Composable
import androidx.compose.ui.test.junit4.createComposeRule
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.Parameterized
import uk.petruha.sshub.phone.core.EnrollChannel
import uk.petruha.sshub.phone.fake.EnrollSamples as S
import uk.petruha.sshub.phone.testing.assertNoVisualOverflow
import uk.petruha.sshub.phone.testing.assertTargets48
import uk.petruha.sshub.phone.testing.setScreen
import uk.petruha.sshub.phone.ui.screens.enroll.BtPhase
import uk.petruha.sshub.phone.ui.screens.enroll.ComparePhase
import uk.petruha.sshub.phone.ui.screens.enroll.ConfirmStatus
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollBtActions
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollBtScreen
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollCompareActions
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollCompareScreen
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollConfirmActions
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollConfirmScreen
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollKeysActions
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollKeysScreen
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollPasteActions
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollPasteScreen
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollScanActions
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollScanScreen
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollShowActions
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollShowScreen
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollSoundActions
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollSoundScreen
import uk.petruha.sshub.phone.ui.screens.enroll.EnrolledActions
import uk.petruha.sshub.phone.ui.screens.enroll.EnrolledScreen
import uk.petruha.sshub.phone.ui.screens.enroll.KeysStatus
import uk.petruha.sshub.phone.ui.screens.enroll.PasteVerdict
import uk.petruha.sshub.phone.ui.screens.enroll.SoundPhase
import uk.petruha.sshub.phone.ui.screens.enroll.SoundProfile

/** Every screen and variant: dark 1.0, light 1.0 and dark 2.0 (spec 4.3). Targets are 48dp and no text overflows. */
private class Variant(val name: String, val content: @Composable () -> Unit)

private val Variants: List<Variant> = listOf(
    Variant("enroll-1") { EnrollScanScreen(S.scan(), EnrollScanActions()) },
    Variant("enroll-1 scanning") { EnrollScanScreen(S.scanning(), EnrollScanActions()) },
    Variant("enroll-1 invalid") { EnrollScanScreen(S.scanInvalid(), EnrollScanActions()) },
    Variant("enroll-1 bluetooth") { EnrollScanScreen(S.scan(EnrollChannel.Bluetooth), EnrollScanActions()) },
    Variant("enroll-1 paste") { EnrollScanScreen(S.scan(EnrollChannel.Paste), EnrollScanActions()) },
    Variant("enroll-1 sound") { EnrollScanScreen(S.scan(EnrollChannel.Sound), EnrollScanActions()) },
    Variant("enroll-2") { EnrollShowScreen(S.show(), EnrollShowActions()) },
    Variant("enroll-2 folded") { EnrollShowScreen(S.show(textOpen = false), EnrollShowActions()) },
    Variant("enroll-2 bluetooth") { EnrollShowScreen(S.show(EnrollChannel.Bluetooth), EnrollShowActions()) },
    Variant("enroll-3 wait") { EnrollKeysScreen(S.keys(KeysStatus.Waiting), EnrollKeysActions()) },
    Variant("enroll-3 ok") { EnrollKeysScreen(S.keys(KeysStatus.Ok), EnrollKeysActions()) },
    Variant("enroll-3 mismatch") { EnrollKeysScreen(S.keys(KeysStatus.Mismatch), EnrollKeysActions()) },
    Variant("enroll-3 bluetooth") { EnrollKeysScreen(S.keys(KeysStatus.Waiting, EnrollChannel.Bluetooth), EnrollKeysActions()) },
    Variant("enroll cmp") { EnrollCompareScreen(S.compare(), EnrollCompareActions()) },
    Variant("enroll ok") { EnrollCompareScreen(S.compare(ComparePhase.Match), EnrollCompareActions()) },
    Variant("enroll diff") { EnrollCompareScreen(S.compare(ComparePhase.Differ), EnrollCompareActions()) },
    Variant("enroll cmp paste") { EnrollCompareScreen(S.compare(channel = EnrollChannel.Paste), EnrollCompareActions()) },
    Variant("enroll-5 waiting") { EnrollConfirmScreen(S.confirm(ConfirmStatus.Waiting), EnrollConfirmActions()) },
    Variant("enroll-5 bad") { EnrollConfirmScreen(S.confirm(ConfirmStatus.Bad), EnrollConfirmActions()) },
    Variant("enroll-5 ok") { EnrollConfirmScreen(S.confirm(ConfirmStatus.Ok), EnrollConfirmActions()) },
    Variant("enroll-done") { EnrolledScreen(S.enrolled(), EnrolledActions()) },
    Variant("enroll-bt searching") { EnrollBtScreen(S.bt(BtPhase.Searching), EnrollBtActions()) },
    Variant("enroll-bt found") { EnrollBtScreen(S.bt(BtPhase.Found, selected = 0), EnrollBtActions()) },
    Variant("enroll-bt mismatch") { EnrollBtScreen(S.bt(BtPhase.Found, selected = 1), EnrollBtActions()) },
    Variant("enroll-bt none") { EnrollBtScreen(S.bt(BtPhase.None, tag = "ZZ9Z"), EnrollBtActions()) },
    Variant("enroll-bt denied") { EnrollBtScreen(S.bt(BtPhase.Denied), EnrollBtActions()) },
    Variant("enroll-bt off") { EnrollBtScreen(S.bt(BtPhase.Off), EnrollBtActions()) },
    Variant("enroll-bt connecting") { EnrollBtScreen(S.bt(BtPhase.Connecting), EnrollBtActions()) },
    Variant("enroll-bt link failed") { EnrollBtScreen(S.bt(BtPhase.LinkFailed), EnrollBtActions()) },
    Variant("enroll-paste 1 empty") { EnrollPasteScreen(S.paste(1, PasteVerdict.Empty), EnrollPasteActions()) },
    Variant("enroll-paste 1 ok") { EnrollPasteScreen(S.paste(1, PasteVerdict.Ok), EnrollPasteActions()) },
    Variant("enroll-paste 1 bad") { EnrollPasteScreen(S.paste(1, PasteVerdict.Bad), EnrollPasteActions()) },
    Variant("enroll-paste 2") { EnrollPasteScreen(S.paste(2), EnrollPasteActions()) },
    Variant("enroll-paste 3 ok") { EnrollPasteScreen(S.paste(3, PasteVerdict.Ok), EnrollPasteActions()) },
    Variant("enroll-paste 5 bad") { EnrollPasteScreen(S.paste(5, PasteVerdict.Bad), EnrollPasteActions()) },
    Variant("enroll-sound idle") { EnrollSoundScreen(S.sound(SoundPhase.Idle), EnrollSoundActions()) },
    Variant("enroll-sound playing") { EnrollSoundScreen(S.sound(SoundPhase.Playing), EnrollSoundActions()) },
    Variant("enroll-sound done") { EnrollSoundScreen(S.sound(SoundPhase.Done), EnrollSoundActions()) },
    Variant("enroll-sound slow") { EnrollSoundScreen(S.sound(SoundPhase.Playing, SoundProfile.Slow), EnrollSoundActions()) },
)

@RunWith(Parameterized::class)
class EnrollMatrixTest(private val index: Int, private val dark: Boolean, private val fontScale: Float) {
    @get:Rule val rule = createComposeRule()

    companion object {
        @JvmStatic
        @Parameterized.Parameters(name = "{index}")
        fun params(): List<Array<Any>> = Variants.indices.flatMap { i ->
            listOf(arrayOf<Any>(i, true, 1f), arrayOf<Any>(i, false, 1f), arrayOf<Any>(i, true, 2f))
        }
    }

    @Test
    fun rendersWithoutOverflowAndWithBigTargets() {
        rule.setScreen(dark, fontScale) { Variants[index].content() }
        rule.waitForIdle()
        rule.assertTargets48()
        rule.assertNoVisualOverflow()
    }
}
