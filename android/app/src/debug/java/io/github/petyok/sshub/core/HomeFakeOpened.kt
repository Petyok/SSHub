package io.github.petyok.sshub.core

/** A non-refusal `Opened` for tests and the debug app (`Opened` is sealed: subtypes must live in this package). */
data class HomeFakeOpened(val label: String) : Opened
