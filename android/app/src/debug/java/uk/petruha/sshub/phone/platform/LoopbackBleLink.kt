package uk.petruha.sshub.phone.platform

/**
 * Test-only link: one message per frame, no chunking, no timers. Two endpoints made by `pair()`: what one sends the other receives
 * once the test moves `takeOutbox()` of one into `onMessage` of the other (or uses `pump()`).
 */
class LoopbackBleLink : BleLink {
    private val outbox = ArrayList<ByteArray>()
    private val frames = ArrayList<ByteArray>()
    override var closed = false
        private set

    override fun onMessage(b: ByteArray) { if (!closed) frames += b }
    override fun takeOutbox(): List<ByteArray> = outbox.toList().also { outbox.clear() }
    override fun sendFrame(f: ByteArray): Boolean { if (closed) return false; outbox += f; return true }
    override fun takeFrames(): List<ByteArray> = frames.toList().also { frames.clear() }
    override fun tick(nowMillis: Long) {}
    fun close() { closed = true }

    companion object {
        fun pair(): Pair<LoopbackBleLink, LoopbackBleLink> = LoopbackBleLink() to LoopbackBleLink()
        /** Moves everything one side has queued to the other side. */
        fun pump(from: LoopbackBleLink, to: LoopbackBleLink) { from.takeOutbox().forEach(to::onMessage) }
    }
}
