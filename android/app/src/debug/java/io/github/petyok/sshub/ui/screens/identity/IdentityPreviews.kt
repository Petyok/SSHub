package io.github.petyok.sshub.ui.screens.identity

import androidx.compose.runtime.Composable
import androidx.compose.ui.tooling.preview.Preview
import io.github.petyok.sshub.fake.IdentitySamples
import io.github.petyok.sshub.ui.theme.SshubTheme

@Composable
private fun Themed(dark: Boolean = true, content: @Composable () -> Unit) = SshubTheme(dark = dark, reducedMotion = true, content = content)

@Preview(widthDp = 412, heightDp = 915) @Composable private fun FirstRunDark() = Themed { FirstRunScreen(IdentitySamples.firstRun, FirstRunActions()) }
@Preview(widthDp = 412, heightDp = 915) @Composable private fun FirstRunNoLockLight() = Themed(false) { FirstRunScreen(IdentitySamples.firstRunNoLock, FirstRunActions()) }
@Preview(widthDp = 412, heightDp = 915) @Composable private fun RestoreBadDark() = Themed { RestoreScreen(IdentitySamples.restoreBad, RestoreActions()) }
@Preview(widthDp = 412, heightDp = 915) @Composable private fun RestoreOverLight() = Themed(false) { RestoreScreen(IdentitySamples.restoreOver, RestoreActions()) }
@Preview(widthDp = 412, heightDp = 915) @Composable private fun BackupDark() = Themed { BackupWordsScreen(IdentitySamples.backupReady, BackupActions()) }
@Preview(widthDp = 412, heightDp = 915) @Composable private fun LockInvalidatedDark() = Themed { LockScreen(IdentitySamples.lockInvalidated, LockActions()) }
@Preview(widthDp = 412, heightDp = 915) @Composable private fun LockLockedOutLight() = Themed(false) { LockScreen(IdentitySamples.lockLockedOut, LockActions()) }
@Preview(widthDp = 412, heightDp = 915) @Composable private fun ReplaceKeyDark() = Themed { ReplaceKeyScreen(IdentitySamples.replaceKeyReady, ReplaceKeyActions()) }
@Preview(widthDp = 412, heightDp = 915) @Composable private fun ChainAgreeDark() = Themed { TakeChainScreen(IdentitySamples.chainHeadAgree, ChainHeadActions()) }
@Preview(widthDp = 412, heightDp = 915) @Composable private fun ChainRefusedLight() = Themed(false) { TakeChainScreen(IdentitySamples.chainHeadRefused, ChainHeadActions()) }
@Preview(widthDp = 412, heightDp = 915) @Composable private fun MenuDark() = Themed { SecurityMenuScreen(IdentitySamples.menu, SecurityMenuActions()) }
@Preview(widthDp = 412, heightDp = 915) @Composable private fun AboutMismatchDark() = Themed { AboutSecurityScreen(IdentitySamples.aboutComparingDiffer, AboutActions()) }
@Preview(widthDp = 412, heightDp = 915) @Composable private fun AboutEqualLight() = Themed(false) { AboutSecurityScreen(IdentitySamples.aboutComparingEqual, AboutActions()) }
