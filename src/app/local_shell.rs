use super::*;

/// Pure. Build the argv for a local-shell session tab.
///
/// Uses `shell_env` (typically `$SHELL`) when it is present and non-empty,
/// falling back to `/bin/sh` otherwise. Returns a single-element argv so the
/// shell launches interactively with no extra arguments.
pub fn local_shell_argv(shell_env: Option<String>) -> Vec<String> {
    let shell = match shell_env {
        Some(s) if !s.is_empty() => s,
        _ => "/bin/sh".to_string(),
    };
    vec![shell]
}

impl App {
    /// Open a session tab running the user's login shell (`$SHELL`, else
    /// `/bin/sh`) instead of ssh. Detach/close semantics are identical to ssh
    /// tabs since it reuses the shared embedded-session machinery.
    pub(crate) fn open_local_shell(&mut self) -> Result<()> {
        let argv = local_shell_argv(std::env::var("SHELL").ok());
        let meta = crate::session::SessionMeta::default();
        self.spawn_embedded_session(argv, "local".into(), meta, None, "local")
    }
}

#[cfg(test)]
mod tests {
    use super::local_shell_argv;

    #[test]
    fn passes_through_non_empty_shell() {
        assert_eq!(
            local_shell_argv(Some("/usr/bin/zsh".to_string())),
            vec!["/usr/bin/zsh".to_string()]
        );
    }

    #[test]
    fn none_falls_back_to_bin_sh() {
        assert_eq!(local_shell_argv(None), vec!["/bin/sh".to_string()]);
    }

    #[test]
    fn empty_string_falls_back_to_bin_sh() {
        assert_eq!(
            local_shell_argv(Some(String::new())),
            vec!["/bin/sh".to_string()]
        );
    }
}
