package io.github.petyok.sshub.fake

import io.github.petyok.sshub.core.*

/** The whole core from the group fakes. Debug app and Compose tests use it; release builds use the uniffi adapter. */
class FakePhoneCore(
    val release: FakeReleaseApi = FakeReleaseApi(),
    val trust: FakeTrust = FakeTrust(),
    val recovery: FakeRecoveryApi = FakeRecoveryApi(),
    val enroll: FakeEnrollApi = FakeEnrollApi(),
    val identity: FakeIdentityApi = FakeIdentityApi(),
    val home: FakeHomeApi = FakeHomeApi(),
    private val chain: FakeChainScan = FakeChainScan(),
) : PhoneCoreApi,
    IdentityApi by identity,
    HomeApi by home,
    ReleaseApi by release,
    TrustApi by trust,
    RecoveryApi by recovery,
    EnrollApi by enroll {

    /**
     * Demo scanning: a pasted or scanned text selects a sample. Words: red, mode, release, bastion, trust, reshare, notice,
     * enroll-code, enroll-keys, enroll-confirm, LIST (own chain), BAD (refused chain list).
     */
    override fun scanGray(width: Int, height: Int, luma: ByteArray): ScanEvent = ScanEvent.None
    override fun resetScan() {}
    override suspend fun frameFromText(text: String): ByteArray? = text.trim().takeIf { it.length >= 3 }?.toByteArray()
    override suspend fun openFrame(frame: ByteArray, now: Long): Opened {
        val t = String(frame)
        return when {
            t.startsWith("enroll-") -> enroll.open(frame)
            t.contains("LIST") || t.contains("BAD") -> chain.openFrame(frame, now)
            t.contains("mode") -> release.pendingView(ReleaseSamples.TOKEN_MODE)
            t.contains("red") -> release.pendingView(ReleaseSamples.TOKEN_REVOKE)
            t.contains("bastion") -> release.pendingView(ReleaseSamples.TOKEN_BASTION)
            t.contains("release") -> release.pendingView(ReleaseSamples.TOKEN_DBS)
            t.contains("trust") -> trust.requests.entries.first().let { OpenedTrustRequest(it.value) }
            t.contains("reshare") -> ReshareOpened(1L, RecoverySamples.candidate)
            t.contains("notice") -> RecoveryNoticeOpened(RecoverySamples.TOKEN, RecoverySamples.noticeView)
            else -> null
        } ?: Opened.Ignored("This is not a frame of this phone.")
    }
}
