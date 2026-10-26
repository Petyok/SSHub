package uk.petruha.sshub.phone.core

import androidx.compose.runtime.staticCompositionLocalOf
import androidx.navigation.NavHostController

/** Everything a screen group's nav builder needs. */
class AppEnv(val core: PhoneCoreApi, val platform: Platform, val nav: NavHostController)

/** Route ids are the screen ids of spec-A section 12. Argument routes are written by the owning group as `Routes.X + "/{arg}"`. */
object Routes {
    const val FirstRun = "firstrun"; const val Restore = "firstrun-restore"; const val Backup = "backup"; const val Lock = "lock"
    const val LockRestore = "lock-restore"; const val ChainHead = "chain-head"
    const val Home = "home"; const val Approvals = "approvals"; const val Scan = "scan"
    const val Red = "red"; const val Release = "release"; const val Refusal = "refusal"
    const val TrustRequest = "trust-request"; const val TrustSigned = "trust-signed"; const val TrustExtend = "trust-extend"
    const val TrustList = "trust-list"; const val TrustRevoked = "trust-revoked"
    const val Codes = "codes"; const val CodeIssued = "code-issued"
    const val Reshare = "reshare"; const val Recovery = "recovery"; const val RecoveryNotice = "recovery-notice"; const val Export = "export"
    const val Enroll1 = "enroll-1"; const val Enroll2 = "enroll-2"; const val Enroll3 = "enroll-3"; const val Enroll4 = "enroll"
    const val Enroll5 = "enroll-5"; const val EnrollDone = "enroll-done"; const val EnrollBt = "enroll-bt"; const val EnrollPaste = "enroll-paste"; const val EnrollSound = "enroll-sound"
    const val Menu = "menu"; const val About = "about"
}

val LocalPlatform = staticCompositionLocalOf<Platform> { error("no Platform") }
