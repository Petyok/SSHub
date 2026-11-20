package io.github.petyok.sshub.home

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.rememberNavController
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import io.github.petyok.sshub.core.AppEnv
import io.github.petyok.sshub.core.LocalPlatform
import io.github.petyok.sshub.core.Routes
import io.github.petyok.sshub.core.ScanPurpose
import io.github.petyok.sshub.fake.FakePhoneCore
import io.github.petyok.sshub.fake.FakePlatform
import io.github.petyok.sshub.fake.ReleaseSamples
import io.github.petyok.sshub.ui.screens.home.homeGraph
import io.github.petyok.sshub.ui.screens.recovery.recoveryGraph
import io.github.petyok.sshub.ui.screens.release.RefusalStore
import io.github.petyok.sshub.ui.screens.release.ReleaseRoutes
import io.github.petyok.sshub.ui.screens.release.releaseGraph
import io.github.petyok.sshub.ui.screens.trust.trustGraph
import io.github.petyok.sshub.ui.theme.SshubTheme

/**
 * H4: every button that opens the scan screen reaches a registered destination with the right purpose. The graphs of all groups are
 * registered as in the app; a bare `navigate("scan")` would throw here.
 */
@RunWith(AndroidJUnit4::class)
class ScanRoutesTest {
    @get:Rule val rule = createComposeRule()
    private lateinit var nav: NavHostController

    private fun host(start: String) = rule.setContent {
        val c = rememberNavController()
        nav = c
        val env = AppEnv(FakePhoneCore(), FakePlatform(), c)
        CompositionLocalProvider(LocalPlatform provides env.platform) {
            SshubTheme(dark = true, reducedMotion = false) {
                NavHost(c, start) { homeGraph(env); trustGraph(env); recoveryGraph(env); releaseGraph(env) }
            }
        }
    }

    private fun assertScan(purpose: ScanPurpose) {
        rule.waitForIdle()
        val e = nav.currentBackStackEntry!!
        assertEquals(Routes.Scan + "/{purpose}", e.destination.route)
        assertEquals(purpose.arg, e.arguments?.getString("purpose"))
    }

    @Test fun trust_sessions_scan_a_request() {
        host(Routes.TrustList)
        rule.onNodeWithText("Scan a request").performClick()
        assertScan(ScanPurpose.Request)
    }

    @Test fun recovery_scan_a_recovery_notice() {
        host(Routes.Recovery)
        rule.onNodeWithText("Scan a recovery notice").performClick()
        assertScan(ScanPurpose.Recovery)
    }

    @Test fun reshare_adopt_a_reshare() {
        host(Routes.Reshare)
        rule.onNodeWithText("Adopt a reshare").performScrollTo().performClick()
        assertScan(ScanPurpose.Reshare)
    }

    @Test fun refusal_scan_again() {
        val id = RefusalStore.put(ReleaseSamples.refusals.first { !it.reason.mismatch })
        host(ReleaseRoutes.refusal(id))
        rule.onNodeWithText("Scan again").performClick()
        assertScan(ScanPurpose.Request)
    }
}
