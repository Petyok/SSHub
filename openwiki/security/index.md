# Files

- [Secrets & Security, credential store, askpass delivery, auth modal, host-key policy, and input safety](secrets.md) - How SSHub handles secrets and security-sensitive behaviour. Profile-namespaced credential store (OS keyring via the keyring crate, credentials.json fallback, lazy opening in the CLI), Secret Service lookup for KeePassXC, the two SSH_ASKPASS delivery paths (staged file and private socket channel) with the jump-host guard, the opt-in remember box, host-key policy per path, OSC 52 relay limits, session-log exposure, file permissions, and the leading-dash target guard.
