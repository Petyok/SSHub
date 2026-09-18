//! Shared whole-command indexing policy. Raw transcript logging remains opt-in
//! and unchanged; future command-input logging must use this classifier too.

/// Maximum indexed command size, measured in UTF-8 bytes.
pub const MAX_COMMAND_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSafety {
    Safe,
    Sensitive { reason: &'static str },
}

/// Reject whole commands rather than retain a partially redacted credential.
/// This is deliberately not a shell parser: expansion, assignments and complex
/// syntax are uncertain and therefore not eligible for indexing.
pub fn classify_command(command: &str) -> CommandSafety {
    let reject = |reason| CommandSafety::Sensitive { reason };
    if command.trim().is_empty() || command.len() > MAX_COMMAND_BYTES {
        return reject("empty or oversized command");
    }
    if command.chars().any(|ch| ch.is_control()) {
        return reject("control character");
    }
    if command.contains(['$', '`', '\\', '=', ';', '|', '&', '<', '>', '(', ')']) {
        return reject("ambiguous shell syntax");
    }
    let mut quote = None;
    for ch in command.chars() {
        if matches!(ch, '\'' | '"') {
            match quote {
                Some(open) if open == ch => quote = None,
                None => quote = Some(ch),
                _ => {}
            }
        }
    }
    if quote.is_some() {
        return reject("unclosed quoting");
    }
    let lower = command.to_ascii_lowercase();
    if [
        "password",
        "passwd",
        "passphrase",
        "token",
        "secret",
        "api_key",
        "api-key",
        "apikey",
        "authorization",
        "bearer",
        "sshpass",
        "credential",
        "private key",
        "private_key",
        "private-key",
    ]
    .iter()
    .any(|pattern| lower.contains(pattern))
    {
        return reject("credential marker");
    }
    // Reject credential-bearing URL forms regardless of protocol.
    if lower.contains("://") && lower.contains('@') {
        return reject("URL credentials");
    }
    // Generic secret-header rule: `curl -H 'X-Api-Key: abc'` carries a
    // credential with no '=' and no listed marker. Any `-H`/`--header` value
    // shaped like `<name>: <value>` is treated as secret-bearing. This
    // deliberately over-redacts benign custom headers: for a secrets
    // classifier, dropping a safe command is the correct direction.
    let mut words = command.split_whitespace();
    while let Some(word) = words.next() {
        let word = word.trim_matches(['\'', '"']);
        if word == "-H" || word == "--header" {
            if words
                .next()
                .is_some_and(|value| value.trim_matches(['\'', '"']).contains(':'))
            {
                return reject("secret header");
            }
        } else if let Some(value) = word.strip_prefix("--header=") {
            if value.trim_matches(['\'', '"']).contains(':') {
                return reject("secret header");
            }
        } else if let Some(value) = word.strip_prefix("-H") {
            // Attached form: `-H'X-Key: abc'`.
            if !value.is_empty() && value.trim_matches(['\'', '"']).contains(':') {
                return reject("secret header");
            }
        }
    }
    let curl = lower
        .split_whitespace()
        .any(|word| word.trim_matches(['\'', '"']).rsplit('/').next() == Some("curl"));
    for word in lower.split_whitespace() {
        let word = word.trim_matches(['\'', '"']);
        if word.starts_with("-p")
            || (curl && word.starts_with("-u"))
            || word.starts_with("--user")
            || word == "login"
        {
            return reject("authentication argument");
        }
    }
    CommandSafety::Safe
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_single_line_commands() {
        for command in [
            "ls -lah /var/log",
            "docker compose logs -f",
            "git status --short",
            "printf '%s' 'hello world'",
            "journalctl -u nginx --since today",
            "curl -H foobar https://example.org",
        ] {
            assert_eq!(classify_command(command), CommandSafety::Safe);
        }
    }

    #[test]
    fn drops_entire_credential_commands() {
        for command in [
            "mysql -pSECRET",
            "MYSQL -pSECRET",
            "curl -u user:password https://example.org",
            "curl -ualice:opaque https://example.org",
            "/usr/bin/curl -u alice:opaque https://example.org",
            "curl --user=user:credential https://example.org",
            "curl -H 'X-Api-Key: abc123' https://example.org",
            "curl -H 'X-Key: abc123' https://example.org",
            "curl --header 'X-Custom-Token: abc' https://example.org",
            "curl --header 'X-Key: abc123' https://example.org",
            "curl -H'X-Key: abc123' https://example.org",
            "APP_TOKEN=abc command",
            "AWS_SECRET_ACCESS_KEY=abc aws s3 ls",
            "docker login -p abc",
            "sshpass -p abc ssh host",
            "tool --password abc",
            "tool api_key=abc",
            "echo https://user:credential@example.org",
            "env SOMETHING=unknown command",
        ] {
            assert!(matches!(
                classify_command(command),
                CommandSafety::Sensitive { .. }
            ));
        }
    }

    #[test]
    fn rejects_ambiguous_shell_syntax_and_controls() {
        for command in [
            "echo 'unterminated",
            "echo $CREDENTIAL",
            "echo `credential-helper`",
            "echo a\\b",
            "echo first\necho second",
            "echo\ttext",
            "echo\0text",
            "echo\u{1b}[31m",
            "echo\u{85}text",
        ] {
            assert!(matches!(
                classify_command(command),
                CommandSafety::Sensitive { .. }
            ));
        }
    }

    #[test]
    fn enforces_byte_limit_and_nonempty_input() {
        assert_eq!(
            classify_command(&"x".repeat(MAX_COMMAND_BYTES)),
            CommandSafety::Safe
        );
        assert!(matches!(
            classify_command(&"x".repeat(MAX_COMMAND_BYTES + 1)),
            CommandSafety::Sensitive { .. }
        ));
        assert!(matches!(
            classify_command("   "),
            CommandSafety::Sensitive { .. }
        ));
        assert!(matches!(
            classify_command(&"é".repeat(MAX_COMMAND_BYTES)),
            CommandSafety::Sensitive { .. }
        ));
    }
}
