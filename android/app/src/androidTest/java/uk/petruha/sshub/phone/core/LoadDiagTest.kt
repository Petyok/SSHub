package uk.petruha.sshub.phone.core

import org.junit.Test

class LoadDiagTest {
    @Test fun load() {
        val bad = listOf("identity.FirstRunAndRestoreTest", "identity.IdentityViewModelsTest").mapNotNull { n ->
            try {
                val c = Class.forName("uk.petruha.sshub.phone.$n")
                c.declaredMethods.size; c.declaredFields.size; c.getDeclaredConstructor().newInstance()
                null
            } catch (t: Throwable) {
                var r: Throwable = t; while (r.cause != null) r = r.cause!!
                "$n: ${r.javaClass.name}: ${r.message}".take(400)
            }
        }
        check(bad.isEmpty()) { bad.joinToString(" | ") }
    }
}
