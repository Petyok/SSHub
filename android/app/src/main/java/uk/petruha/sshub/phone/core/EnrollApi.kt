package uk.petruha.sshub.phone.core

/*
 * Enrollment of this phone into a terminal's ring (joining side, role 0). Frames come from any channel through `ScanApi.openFrame`
 * and are answered by `enrollDecide`. The core owns every check (commitment, key match, SAS words, signatures); the UI renders
 * the views below as they are.
 *
 *   kind 6 (terminal short code)  -> OpenedEnroll(Commit)   -> enrollDecide(Name)  -> Outgoing = this phone's commitment
 *   kind 7 (terminal keys)        -> OpenedEnroll(Reveal)   -> enrollDecide(Sas)   -> Outgoing = confirmation (or the decline)
 *   kind 8 (terminal confirmation)-> OpenedEnrolled
 *   a failed check inside a session -> EnrollRefusal (a value, never an Approve button)
 */

enum class EnrollChannel { Qr, Bluetooth, Paste, Sound }

enum class EnrollStage { Commit, Reveal }

/**
 * Frame of kind 6 (`Commit`: terminal id and name known, `sasWords` empty, `outgoing` null) or kind 7 (`Reveal`: the four SAS
 * words, and `outgoing` = this phone's own keys, which the terminal needs while the user compares the words).
 */
data class OpenedEnroll(
    val token: Long,
    val stage: EnrollStage,
    val sasWords: List<String>,
    val terminalId: String,
    val terminalName: String,
    val outgoing: Outgoing?,
) : Opened

/** Kind 8: the terminal's confirmation is received and its signature is good. */
data class OpenedEnrolled(
    val terminalId: String,
    val terminalName: String,
    val phoneName: String,
    val phoneId: String,
) : Opened

/**
 * `Mismatch`: the keys do not open the first code (stop box with both values when the core gave them). `Stopped`: another refusal that
 * ended the session (the sentence is the headline). `Stray`: a frame the session waits through, shown as a note only.
 */
enum class RefusalKind { Mismatch, Stopped, Stray }

/**
 * A check failed inside an enrollment session. `committed` and `shown` are the two compared values in full when the keys do not
 * match the first code (otherwise null); `reason` is literal text.
 */
data class EnrollRefusal(val reason: String, val committed: String?, val shown: String?, val kind: RefusalKind = RefusalKind.Stopped) : Opened

sealed interface EnrollDecision {
    /** Step 1 to 2: the name this phone will have in the ring. */
    data class Name(val name: String) : EnrollDecision
    /** Step 4: the user compared the four words. `matches = false` produces the declined confirmation for the terminal. */
    data class Sas(val matches: Boolean) : EnrollDecision
}

/** What pasted text is, judged by the core (the paste screens show it; they never decide it). */
enum class EnrollTextKind { Empty, Step1Code, Step3Keys, Step5Confirm, OwnText, NotEnrollment }

interface EnrollApi {
    /** Null when the name is acceptable; otherwise the literal reason to show. */
    fun nameProblem(name: String): String?
    suspend fun enrollTextKind(text: String): EnrollTextKind
    suspend fun enrollDecide(token: Long, decision: EnrollDecision, now: Long): Outgoing
}
