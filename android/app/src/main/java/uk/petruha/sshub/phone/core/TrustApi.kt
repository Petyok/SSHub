package uk.petruha.sshub.phone.core

/**
 * Trust group: trust sessions (certificates this phone signs for a terminal), budget extensions, issued certificates, short codes.
 *
 * Errors are `CoreException` (Locked/BadInput/Refused/Internal mapped onto `CoreException.Code`). A refusal of a REQUEST is not an
 * error: it is a value inside the view (`TrustRequestView.refusal`, `TrustExtendView.refusal`) and such a screen has no Approve.
 * Every "what you are approving" string (fields, headline, checks) is data from the core and is rendered as is.
 */
interface TrustApi {
    /** The request that `openFrame` returned as `Opened.TrustRequest` (the core keeps it under its token); null when the token is unknown or lapsed. */
    suspend fun trustRequest(token: Long): TrustRequestView?

    /** Same for `Opened.TrustExtend`. */
    suspend fun trustExtend(token: Long): TrustExtendView?

    /**
     * Sign or refuse a trust request or an extension. `Approve` returns the signed hand-off; `Decline` signs nothing and returns null.
     * For a requester never met the core checks `Approve.typedId` itself; the UI only gates the button.
     */
    suspend fun decideTrust(token: Long, decision: TrustDecision, now: Long): Outgoing?

    /** Certificates this phone issued, newest first. */
    suspend fun issuedCerts(now: Long): List<IssuedCert>

    /** Signs a revocation of an active certificate. */
    suspend fun revokeCert(certId: String, now: Long): Outgoing

    /** Short codes this phone issued (the code itself is never stored). */
    suspend fun shortCodes(now: Long): List<ShortCodeRow>

    /** `host` is the credential name of the row, e.g. `deploy@db-staging`. */
    suspend fun declareShortCodeDead(host: String)

    /**
     * The short code that was just issued, or the refusal when one is live. Returns it ONCE: the core forgets it on the first call,
     * so it exists only on the screen that shows it. Null when nothing is pending.
     */
    suspend fun takeIssuedShortCode(now: Long): CodeIssuedView?
}
