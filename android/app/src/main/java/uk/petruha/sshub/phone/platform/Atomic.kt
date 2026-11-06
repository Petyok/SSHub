package uk.petruha.sshub.phone.platform

import android.system.Os
import android.system.OsConstants
import java.io.File
import java.io.FileOutputStream
import java.nio.file.Files
import java.nio.file.StandardCopyOption

/*
 * Every function in this file blocks on disk IO (fsync included): callers must be on Dispatchers.IO.
 */

/**
 * Writes `bytes` to `target` so a reader sees either the old file or the new one: temp file next to it, fsync, rename, fsync of the directory.
 * A torn `<name>.tmp` left by a crash is never read; the next write overwrites it.
 */
fun atomicWrite(target: File, bytes: ByteArray) {
    val dir = target.absoluteFile.parentFile!!
    dir.mkdirs()
    val tmp = File(dir, target.name + ".tmp")
    FileOutputStream(tmp).use { it.write(bytes); it.fd.sync() }
    atomicMove(tmp, target)
}

/** Renames `from` over `to` atomically (replacing it) and fsyncs the directory. Same directory, same filesystem. */
fun atomicMove(from: File, to: File) {
    Files.move(from.toPath(), to.absoluteFile.toPath(), StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
    runCatching {
        val fd = Os.open(to.absoluteFile.parentFile!!.path, OsConstants.O_RDONLY, 0)
        try { Os.fsync(fd) } finally { Os.close(fd) }
    }
}

fun atomicRead(target: File): ByteArray? = if (target.isFile) target.readBytes() else null

/** The sealed state blob of the core (docs 2.1): one file, atomic replace. The bytes are already sealed by the core; this only stores them. */
class SealedStateStore(dir: File, name: String = "state.bin") {
    private val file = File(dir, name)
    fun read(): ByteArray? = atomicRead(file)
    fun write(blob: ByteArray) = atomicWrite(file, blob)
    /** Keeps the current bytes as `<name>.lost`, or `.lost.1`, `.lost.2`... for the next free name: an earlier lost copy is never overwritten. Blocking. */
    fun keepAsLost() {
        if (!file.isFile) return
        val base = File(file.parentFile, file.name + ".lost")
        var target = base
        var n = 0
        while (target.exists()) target = File(file.parentFile, base.name + "." + ++n)
        atomicWrite(target, file.readBytes())
    }
    fun delete() { file.delete(); File(file.parentFile, file.name + ".tmp").delete() }
}
