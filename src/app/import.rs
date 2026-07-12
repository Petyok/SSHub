use super::*;

impl App {
    /// Import hosts from ssh config into the launcher store (`source=ssh_config`).
    pub fn import_ssh_config(&mut self) -> Result<ImportReport> {
        let report =
            import_ssh_config(self.resolver.as_ref(), &self.store, self.metadata.as_ref())?;
        self.reload_hosts()?;
        Ok(report)
    }

    /// Open the Termius CSV import prompt (asks for the export directory).
    pub fn open_import_prompt(&mut self) {
        let path = crate::import::termius_csv::default_export_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let cursor = path.chars().count();
        self.import_prompt = Some(ImportPromptEdit {
            path,
            cursor,
            error: None,
        });
        self.mode = AppMode::ImportPrompt;
    }

    pub(crate) fn import_prompt_insert(&mut self, ch: char) {
        if let Some(prompt) = self.import_prompt.as_mut() {
            prompt.cursor = text_input::insert_at(&mut prompt.path, prompt.cursor, ch);
            prompt.error = None;
        }
    }

    pub(crate) fn import_prompt_backspace(&mut self) {
        if let Some(prompt) = self.import_prompt.as_mut() {
            prompt.cursor = text_input::backspace_at(&mut prompt.path, prompt.cursor);
            prompt.error = None;
        }
    }

    pub(crate) fn handle_key_import_prompt(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.import_prompt = None;
                self.mode = AppMode::Normal;
            }
            KeyCode::Enter | KeyCode::F(2) => self.run_termius_import()?,
            KeyCode::Backspace if key.modifiers.is_empty() => self.import_prompt_backspace(),
            KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End | KeyCode::Delete => {
                if let Some(p) = self.import_prompt.as_mut() {
                    let mut cursor = p.cursor;
                    text_input::handle_cursor_key(key.code, &mut p.path, &mut cursor);
                    p.cursor = cursor;
                    if key.code == KeyCode::Delete {
                        p.error = None;
                    }
                }
            }
            KeyCode::Char(c)
                if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                    && !c.is_control() =>
            {
                self.import_prompt_insert(c);
            }
            _ => {}
        }
        Ok(())
    }

    /// Run the Termius CSV import using the path entered in the prompt.
    pub(crate) fn run_termius_import(&mut self) -> Result<()> {
        let Some(prompt) = self.import_prompt.as_ref() else {
            return Ok(());
        };
        let raw = prompt.path.trim();
        if raw.is_empty() {
            if let Some(p) = self.import_prompt.as_mut() {
                p.error = Some("Enter the Termius export folder path".into());
            }
            return Ok(());
        }

        // Accept a path pointing directly at L00t.csv by using its parent folder.
        let mut dir = shellexpand_home(raw);
        if dir.is_file() {
            if let Some(parent) = dir.parent() {
                dir = parent.to_path_buf();
            }
        }

        match crate::import::termius_csv::import_csv_export(
            &dir,
            &self.store,
            self.password_store.as_ref(),
        ) {
            Ok(report) => {
                let mut msg = format!(
                    "Termius: {} hosts new, {} skipped · {} passwords + {} passphrases stored",
                    report.hosts_imported,
                    report.skipped,
                    report.passwords_stored,
                    report.passphrases_stored,
                );
                if report.identities_created > 0 {
                    msg.push_str(&format!(" · {} new keys", report.identities_created));
                }
                if report.keyring_failures > 0 {
                    msg.push_str(&format!(
                        " · ⚠ {} keyring writes failed verification",
                        report.keyring_failures
                    ));
                }
                self.host_notice = Some(msg);
                self.import_prompt = None;
                self.mode = AppMode::Normal;
                self.reload_hosts()?;
            }
            Err(e) => {
                // Keep the prompt open and show why, so the user can fix the path.
                if let Some(p) = self.import_prompt.as_mut() {
                    p.error = Some(format!("{e:#}"));
                }
            }
        }
        Ok(())
    }

    /// Export launcher-native hosts to `config_dir/exported.conf`.
    pub fn export_ssh_config(&mut self) -> Result<std::path::PathBuf> {
        export_launcher_hosts(&self.store)
    }
}
