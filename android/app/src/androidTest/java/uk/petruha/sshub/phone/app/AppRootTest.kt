package uk.petruha.sshub.phone.app

import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.and
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uk.petruha.sshub.phone.fake.FakePhoneCore
import uk.petruha.sshub.phone.fake.FakePlatform

/** The app shell over the Kotlin fakes: start destination, the four tabs, the bar, and the lock gate. Exercises the nav graphs of all groups together. */
@RunWith(AndroidJUnit4::class)
class AppRootTest {
    @get:Rule val rule = createComposeRule()

    private fun tab(label: String) = rule.onNode(hasText(label) and SemanticsMatcher.expectValue(SemanticsProperties.Role, Role.Tab))

    private fun start(platform: FakePlatform = FakePlatform()) = rule.setContent { AppRoot(FakePhoneCore(), platform) }

    @Test fun an_unlocked_vault_starts_on_home_with_the_four_tabs() {
        start()
        for (t in listOf("Ring", "Approvals", "Enroll", "Security")) tab(t).assertExists()
    }

    @Test fun tabs_navigate_to_their_roots() {
        start()
        tab("Approvals").performClick()
        rule.onNodeWithText("Scan a request").assertExists()
        tab("Security").performClick()
        rule.onNodeWithText("Lock now").performScrollTo().assertExists()
        tab("Enroll").performClick()
        rule.onNodeWithText("Scan the terminal's code").assertExists()
    }

    @Test fun a_missing_key_starts_the_first_run_without_the_bar() {
        val p = FakePlatform()
        p.keys.st.value = uk.petruha.sshub.phone.core.VaultState.NoKey
        start(p)
        rule.onAllNodes(hasText("Approvals") and SemanticsMatcher.expectValue(SemanticsProperties.Role, Role.Tab)).assertCountEquals(0)
    }
}
