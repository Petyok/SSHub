package uk.petruha.sshub.phone.enroll

import androidx.navigation.NavHostController
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uk.petruha.sshub.phone.core.EnrollChannel
import uk.petruha.sshub.phone.core.EnrollRefusal
import uk.petruha.sshub.phone.core.Opened
import uk.petruha.sshub.phone.core.PhoneCoreApi
import uk.petruha.sshub.phone.core.RefusalKind
import uk.petruha.sshub.phone.fake.FakePhoneCore
import uk.petruha.sshub.phone.fake.FakePlatform
import uk.petruha.sshub.phone.ui.screens.enroll.ConfirmStatus
import uk.petruha.sshub.phone.ui.screens.enroll.EnrollViewModel
import uk.petruha.sshub.phone.ui.screens.enroll.KeysStatus

/** The core keeps an enrollment going after a wrong-order or stray frame; the screens keep the step and show a note. */
private class RefusingCore(private val inner: FakePhoneCore = FakePhoneCore()) : PhoneCoreApi by inner {
    var refuse: RefusalKind? = null
    override suspend fun openFrame(frame: ByteArray, now: Long): Opened =
        refuse?.let { EnrollRefusal("refused by the core", null, null, it) } ?: inner.openFrame(frame, now)
}

class EnrollRefusalFlowTest {
    private val inst = InstrumentationRegistry.getInstrumentation()
    private val core = RefusingCore()
    private val note = "That is not the code this step needs. Show the one the terminal shows now."

    private fun onMain(block: () -> Unit) { inst.runOnMainSync(block); inst.waitForIdleSync() }

    private fun waitFor(what: String, cond: () -> Boolean) {
        val end = System.currentTimeMillis() + 5_000
        while (!cond() && System.currentTimeMillis() < end) Thread.sleep(25)
        assertTrue(what, cond())
    }

    private fun vm(): EnrollViewModel {
        lateinit var v: EnrollViewModel
        onMain { v = EnrollViewModel(core, FakePlatform(), NavHostController(inst.targetContext), "pixel-8"); v.setChannel(EnrollChannel.Paste) }
        return v
    }

    /** Pastes `text` for the current paste step; the paste channel has no navigation at step 1, the sound channel none at steps 3 to 5. */
    private fun paste(v: EnrollViewModel, text: String) { onMain { v.setPasteText(text); v.pasteUse() } }

    private fun toStep3(v: EnrollViewModel) {
        paste(v, "enroll-code")
        waitFor("commit made") { v.ui.value.flow.commit != null }
        onMain { v.pasteNext() }
    }

    @Test fun a_stray_frame_at_step_3_is_a_note_and_the_step_keeps_its_state() {
        val v = vm(); toStep3(v)
        core.refuse = RefusalKind.StrayFrame
        paste(v, "enroll-keys")
        waitFor("note shown") { v.ui.value.flow.note == note }
        val f = v.ui.value.flow
        assertEquals(KeysStatus.Waiting, f.keys)
        assertNotNull(f.commit)
        assertNull(f.reveal)
    }

    @Test fun a_wrong_order_frame_at_step_3_is_the_same_note() {
        val v = vm(); toStep3(v)
        core.refuse = RefusalKind.WrongOrder
        paste(v, "enroll-keys")
        waitFor("note shown") { v.ui.value.flow.note == note }
        assertEquals(KeysStatus.Waiting, v.ui.value.flow.keys)
    }

    @Test fun a_mismatch_at_step_3_still_ends_the_step() {
        val v = vm(); toStep3(v)
        core.refuse = RefusalKind.Mismatch
        onMain { v.setChannel(EnrollChannel.Sound) } // the paste channel would navigate to the mismatch screen
        paste(v, "enroll-keys")
        waitFor("mismatch") { v.ui.value.flow.keys == KeysStatus.Mismatch }
    }

    private fun toStep5(v: EnrollViewModel) {
        toStep3(v)
        onMain { v.setChannel(EnrollChannel.Sound) } // no navigation at the reveal or the decision
        paste(v, "enroll-keys")
        waitFor("reveal") { v.ui.value.flow.reveal != null || v.ui.value.flow.revealToken != null }
        onMain { v.decideSas(true) }
        waitFor("confirm made") { v.ui.value.flow.confirm != null }
    }

    @Test fun a_stray_frame_at_step_5_is_a_note_and_the_confirmation_stays_open() {
        val v = vm(); toStep5(v)
        core.refuse = RefusalKind.StrayFrame
        paste(v, "enroll-confirm")
        waitFor("note shown") { v.ui.value.flow.note == note }
        val f = v.ui.value.flow
        assertEquals(ConfirmStatus.Waiting, f.conf)
        assertNull(f.confKind)
        assertNotNull(f.confirm)
    }

    @Test fun a_bad_signature_at_step_5_ends_it_with_its_kind() {
        val v = vm(); toStep5(v)
        core.refuse = RefusalKind.BadSignature
        paste(v, "enroll-confirm")
        waitFor("bad") { v.ui.value.flow.conf == ConfirmStatus.Bad }
        assertEquals(RefusalKind.BadSignature, v.ui.value.flow.confKind)
    }
}
