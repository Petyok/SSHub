package uk.petruha.sshub.phone.core

/** Result of the core's word-list check for one typed word. */
enum class WordState {
    Empty,
    /** Not a word yet, but the start of one. */
    Partial,
    Ok,
    /** Not in the list and not the start of a word. */
    Bad,
}

/** One line of the checks on a head list. `ok == null` means the check did not run (an earlier one failed). Text comes from the core. */
data class ChainCheck(val text: String, val ok: Boolean?)

/** The core's record of one own-ops list (frame kind 5). `agreed` is true once two lists end at the same newest op. */
data class ChainAdopted(
    val token: Long,
    val agreed: Boolean,
    val headSeq: Long,
    /** Signing time of the newest op, unix seconds. */
    val newestAt: Long,
    val ops: Int,
    /** Newest seq this phone knew before, null when it knew none. */
    val previousSeq: Long?,
    /** The core's sentence under the title ("Newest op in this list: seq N, signed ... (n ops read)"); empty: no line. */
    val summary: String = "",
    /** What to do next, in the core's words (take another list, or the warning to read before adopting); empty: no line. */
    val note: String = "",
    /** Id of the terminal that made the list (unsigned, so a claim and not proof); null when the list did not say. */
    val sender: String? = null,
    /** The head came from ONE list: accepting it needs the user's tick (`adoptChain(.., acknowledgedSingleSource = true)`). */
    val singleSource: Boolean = false,
    /** The sending terminal's own statement of how many terminals the ring has, and the signing time of the newest op it holds; unsigned, informational. */
    val terminalsCount: Int? = null,
    val terminalsNewestAt: Long? = null,
)

/** `openFrame` of a kind 5 own-ops list that verified. Top level because `Opened` is sealed in Core.kt. */
data class OpenedChain(val chain: ChainAdopted, val checks: List<ChainCheck>) : Opened

/* A kind 5 list that does not verify comes back as the release group's `OpenedRefused` (core/ReleaseModels.kt); the chain-head screen renders it itself. */

/** Outcome of accepting a head. `headSeq` is null when the user confirmed "never signed" instead of taking a chain. */
data class ChainTaken(val headSeq: Long?, val status: PhoneStatus)
