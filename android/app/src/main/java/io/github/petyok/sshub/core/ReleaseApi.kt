package io.github.petyok.sshub.core

/** Owned by the Release screen group: Red operations, share releases, refusals. */
interface ReleaseApi {
    /** The view `openFrame` produced for `token` (OpenedRed or OpenedRelease), or null when the core no longer holds it. */
    fun pendingView(token: Long): Opened?

    /**
     * Approve or decline. Returns only after the core produced the bytes. Throws CoreException (Refused, Expired, Internal);
     * a requester never met with a wrong `typedId` is a Refused error, whatever the UI did.
     */
    suspend fun decide(token: Long, decision: ReleaseDecision, now: Long): ReleaseOutcome

    /** The outcome of the last successful Approve for `token`, kept until the phone locks, so a recreated screen can show it again. Null for none. */
    fun lastOutcome(token: Long): ReleaseOutcome? = null
}
