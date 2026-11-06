package uk.petruha.sshub.phone.live

import android.content.ClipboardManager
import android.content.Context
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isEnabled
import androidx.compose.ui.test.isRoot
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import org.json.JSONObject
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uk.petruha.sshub.phone.MainActivity
import java.io.File

/**
 * A live session of the REAL app (real MainActivity, UniffiPhoneCore, AndroidPlatform, Keystore, BiometricPrompt) driven from outside by
 * the e2e scenario `66-app-live-terminal.sh`. There is one long test, `serve`: it polls `<external files>/live/in/*.json`, runs each
 * command against the UI through its semantics (texts and content descriptions, no test tags), and writes `live/out/<same name>.json`.
 * The scenario moves the files with `adb shell`; the frames of the terminal come in through the app's paste boxes and the frames of
 * the app go out through the text it shows or the clipboard its Copy buttons fill (QR, sound and Bluetooth are not used here).
 *
 * Commands (JSON object, field `op`): screen, click {text, exact, index}, type {desc, text}, read {desc}, find {pattern},
 * wait {text}, exists {text}, auth, setpin, shell {cmd}, clip, back, settle, stop. Every answer has `ok`; a failure carries `error`
 * and a dump of the screen.
 */
@RunWith(AndroidJUnit4::class)
class LiveTerminalTest {
    @get:Rule val rule = createAndroidComposeRule<MainActivity>()

    private val inst = InstrumentationRegistry.getInstrumentation()
    private val ctx: Context = inst.targetContext
    private val dev: UiDevice = UiDevice.getInstance(inst)
    private val dir = File(ctx.getExternalFilesDir(null), "live")
    private var pin = "1234"

    @Test fun serve() {
        File(dir, "in").mkdirs()
        File(dir, "out").mkdirs()
        File(dir, "ready").writeText(dir.absolutePath)
        val end = System.currentTimeMillis() + 120 * 60_000L
        while (System.currentTimeMillis() < end) {
            val next = File(dir, "in").listFiles { f -> f.name.endsWith(".json") }?.minByOrNull { it.name }
            if (next == null) { Thread.sleep(250); continue }
            val text = next.readText()
            next.delete()
            val stop = text.contains("\"stop\"")
            val answer = try {
                handle(JSONObject(text))
            } catch (e: Throwable) {
                JSONObject().put("ok", false).put("error", e.toString().take(600)).put("screen", dumpSafely())
            }
            val id = next.name.removeSuffix(".json")
            val tmp = File(dir, "out/$id.tmp")
            tmp.writeText(answer.toString())
            tmp.renameTo(File(dir, "out/$id.json"))
            if (stop) return
        }
    }

    private fun handle(c: JSONObject): JSONObject {
        val r = JSONObject().put("ok", true)
        when (c.getString("op")) {
            "screen" -> r.put("screen", dump())
            "click" -> click(c.getString("text"), c.optBoolean("exact", true), c.optInt("index", 0), c.optLong("timeout", 20_000))
            "type" -> type(c.getString("desc"), c.getString("text"), c.optLong("timeout", 20_000))
            "read" -> r.put("text", read(c.getString("desc"), c.optLong("timeout", 20_000)))
            "find" -> r.put("match", find(c.getString("pattern"), c.optLong("timeout", 20_000)))
            "wait" -> { val t = c.getString("text"); waitFor(c.optLong("timeout", 30_000), "text '$t'") { strings().any { it.contains(t) } } }
            "exists" -> r.put("exists", strings().any { it.contains(c.getString("text")) })
            "auth" -> r.put("auth", auth(c.optLong("timeout", 40_000)))
            "setpin" -> {
                pin = c.optString("pin", pin)
                r.put("out", dev.executeShellCommand("locksettings set-pin $pin"))
            }
            "shell" -> r.put("out", dev.executeShellCommand(c.getString("cmd")))
            "clip" -> r.put("text", clip())
            "back" -> dev.pressBack()
            "settle" -> rule.waitForIdle()
            "stop" -> Unit
            else -> error("unknown op ${c.getString("op")}")
        }
        return r
    }

    // ---- reading the screen -------------------------------------------------------------------------------------------------

    private fun roots(): List<SemanticsNode> = rule.onAllNodes(isRoot(), useUnmergedTree = true).fetchSemanticsNodes()

    private fun visit(n: SemanticsNode, depth: Int, out: (SemanticsNode, Int) -> Unit) {
        out(n, depth)
        n.children.forEach { visit(it, depth + 1, out) }
    }

    /** Every text, content description and edited text on the screen (unmerged tree). */
    private fun strings(): List<String> {
        val all = mutableListOf<String>()
        roots().forEach { root ->
            visit(root, 0) { n, _ ->
                n.config.getOrNull(SemanticsProperties.Text)?.forEach { all += it.text }
                n.config.getOrNull(SemanticsProperties.ContentDescription)?.forEach { all += it }
                n.config.getOrNull(SemanticsProperties.EditableText)?.let { all += it.text }
            }
        }
        return all
    }

    private fun dump(): String {
        val sb = StringBuilder()
        sb.appendLine("focus=${hasFocus()}")
        roots().forEach { root ->
            visit(root, 0) { n, d ->
                val t = n.config.getOrNull(SemanticsProperties.Text)?.joinToString("|") { it.text }
                val e = n.config.getOrNull(SemanticsProperties.EditableText)?.text
                val cd = n.config.getOrNull(SemanticsProperties.ContentDescription)?.joinToString("|")
                if (t != null || e != null || cd != null) {
                    sb.append("  ".repeat(d.coerceAtMost(6)))
                    if (t != null) sb.append("t=").append(t.take(160)).append(' ')
                    if (e != null) sb.append("e=").append(e.take(60)).append(' ')
                    if (cd != null) sb.append("d=").append(cd.take(160)).append(' ')
                    if (n.config.contains(SemanticsActions.OnClick)) sb.append("[click]")
                    sb.appendLine()
                }
            }
        }
        return sb.toString().take(7000)
    }

    private fun dumpSafely(): String = try { dump() } catch (e: Throwable) { "no dump: $e" }

    private fun hasFocus(): Boolean {
        var f = false
        inst.runOnMainSync { f = rule.activity.hasWindowFocus() }
        return f
    }

    private fun waitFor(timeoutMs: Long, what: String, cond: () -> Boolean) {
        val end = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < end) {
            if (cond()) return
            Thread.sleep(200)
        }
        error("timeout after $timeoutMs ms waiting for $what; screen:\n" + dumpSafely())
    }

    // ---- acting ---------------------------------------------------------------------------------------------------------------

    private fun click(text: String, exact: Boolean, index: Int, timeoutMs: Long) {
        val m = hasClickAction() and hasText(text, substring = !exact) and isEnabled()
        waitFor(timeoutMs, "an enabled button '$text'") { rule.onAllNodes(m).fetchSemanticsNodes().size > index }
        val node = rule.onAllNodes(m)[index]
        runCatching { node.performScrollTo() }
        node.performClick()
        rule.waitForIdle()
    }

    private fun type(desc: String, text: String, timeoutMs: Long) {
        val m = hasContentDescription(desc)
        waitFor(timeoutMs, "a field '$desc'") { rule.onAllNodes(m).fetchSemanticsNodes().isNotEmpty() }
        val node = rule.onAllNodes(m).onFirst()
        runCatching { node.performScrollTo() }
        node.performTextReplacement(text)
        rule.waitForIdle()
    }

    private fun read(desc: String, timeoutMs: Long): String {
        val m = hasContentDescription(desc)
        waitFor(timeoutMs, "a node '$desc'") { rule.onAllNodes(m).fetchSemanticsNodes().isNotEmpty() }
        val n = rule.onAllNodes(m).fetchSemanticsNodes().first()
        return n.config.getOrNull(SemanticsProperties.EditableText)?.text
            ?: n.config.getOrNull(SemanticsProperties.Text)?.joinToString("") { it.text }
            ?: n.config.getOrNull(SemanticsProperties.ContentDescription)?.joinToString(", ")
            ?: ""
    }

    private fun find(pattern: String, timeoutMs: Long): String {
        val re = Regex(pattern)
        var hit: String? = null
        waitFor(timeoutMs, "a text matching /$pattern/") {
            hit = strings().firstNotNullOfOrNull { s -> re.find(s)?.let { m -> if (m.groupValues.size > 1) m.groupValues[1] else m.value } }
            hit != null
        }
        return hit!!
    }

    private fun clip(): String {
        var text = ""
        inst.runOnMainSync {
            val cm = ctx.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            text = cm.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(ctx)?.toString() ?: ""
        }
        return text
    }

    /**
     * The system authentication prompt (BiometricPrompt with the screen lock as the credential) takes the window focus from the activity.
     * If it shows up within a few seconds, the PIN is typed with the shell input and the focus is waited for again.
     */
    private fun auth(timeoutMs: Long): String {
        val start = System.currentTimeMillis()
        var typed = false
        while (System.currentTimeMillis() - start < timeoutMs) {
            val focus = hasFocus()
            if (!focus && !typed) {
                Thread.sleep(2000)
                dev.executeShellCommand("input text $pin")
                Thread.sleep(500)
                dev.executeShellCommand("input keyevent 66")
                typed = true
            } else if (focus && typed) {
                rule.waitForIdle()
                return "authenticated"
            } else if (focus && System.currentTimeMillis() - start > 6_000) {
                return "no prompt"
            }
            Thread.sleep(300)
        }
        error("authentication did not finish in $timeoutMs ms (typed=$typed); screen:\n" + dumpSafely())
    }
}
