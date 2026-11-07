package uk.petruha.sshub.phone.core

/** A non-refusal `Opened` for tests and the debug app (`Opened` is sealed: subtypes must live in this package). */
data class HomeFakeOpened(val label: String) : Opened
