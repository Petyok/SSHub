package uk.petruha.sshub.phone.core

/**
 * Recovery screen group: wrap keys and reshare, recovery notices, export of the newest ops.
 * Every string that says what a request does comes from the core as data; the UI renders it as is. `now` is unix seconds.
 * Methods that sign are suspend and return after the core produced the bytes.
 */
interface RecoveryApi {
    suspend fun wrapKeyState(now: Long): WrapKeyState

    /** Rotates the wrap keys and appends the signed op to this phone chain. The new state is read with `wrapKeyState`. */
    suspend fun rotateWrapKeys(now: Long): Outgoing

    /** The reshare that `openFrame` returned as [ReshareOpened] under `token`, for review before it is adopted. Null when unknown. */
    suspend fun reshareCandidate(token: Long, now: Long): ReshareView?

    /** The user accepts the reshare shown under `token`: the core adopts it and signs the new shares in ONE call. */
    suspend fun decideReshare(token: Long, now: Long): ReshareOutcome

    /** The newest 8 signed ops of this phone chain as rows, and as text and QR. */
    suspend fun exportOps(now: Long): ExportedOps

    /** Recovery notices this phone knows about (from synced chains or scans) that are still inside their timelock. */
    suspend fun recoveryNotices(now: Long): List<RecoveryNoticeSummary>

    /** The notice that `openFrame` returned as [RecoveryNoticeOpened], or one listed by `recoveryNotices`. Null when unknown. */
    suspend fun recoveryNotice(token: Long, now: Long): RecoveryNoticeView?

    /** Signs the cancellation of the recovery under `token`. The signed cancellation is handed to a terminal as QR and text. */
    suspend fun decideRecoveryCancel(token: Long, now: Long): Outgoing
}
