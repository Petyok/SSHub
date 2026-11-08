package uk.petruha.sshub.phone.enroll

import androidx.compose.ui.test.junit4.createComposeRule
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import uk.petruha.sshub.phone.core.EnrollChannel
import uk.petruha.sshub.phone.core.Routes
import uk.petruha.sshub.phone.fake.FakePhoneCore
import uk.petruha.sshub.phone.fake.FakePlatform
import uk.petruha.sshub.phone.ui.screens.enroll.ComparePhase
import uk.petruha.sshub.phone.ui.screens.enroll.ConfirmStatus
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollViewModel

/** Scenario 66: the paste channel ends on the Finish screen (step 5) like QR, not straight on "Enrolled". */
class EnrollPasteFinishTest {
    @get:Rule val rule = createComposeRule()
    private lateinit var nav: NavHostController
    private lateinit var vm: EnrollViewModel

    private fun route() = nav.currentDestination?.route

    private fun paste(text: String, until: () -> Boolean) {
        rule.runOnIdle { vm.setPasteText(text); vm.pasteUse() }
        rule.waitUntil(5_000) { until() }
    }

    @Test fun paste_reaches_the_finish_screen_before_the_done_screen() {
        rule.setContent {
            val c = rememberNavController()
            nav = c
            NavHost(c, "start") {
                composable("start") {}
                listOf(Routes.EnrollPaste, Routes.Enroll4, Routes.Enroll5, Routes.EnrollDone).forEach { composable(it) {} }
            }
        }
        rule.runOnIdle {
            vm = EnrollViewModel(FakePhoneCore(), FakePlatform(), nav, "pixel-8")
            vm.setChannel(EnrollChannel.Paste)
            vm.startPaste()
        }
        paste("enroll-code") { vm.ui.value.flow.commit != null }
        rule.runOnIdle { vm.pasteNext() }
        paste("enroll-keys") { route() == Routes.Enroll4 }
        rule.runOnIdle { vm.decideSas(true) }
        rule.waitUntil(5_000) { vm.ui.value.flow.cmp == ComparePhase.Match }
        rule.runOnIdle { vm.continueToConfirm() }
        paste("enroll-confirm") { vm.ui.value.flow.conf == ConfirmStatus.Ok }
        rule.waitUntil(5_000) { route() == Routes.Enroll5 }
        assertEquals(Routes.Enroll5, route())
        rule.runOnIdle { vm.go(Routes.EnrollDone) }
        rule.waitUntil(5_000) { route() == Routes.EnrollDone }
    }
}
