//! Three small modals every admin flow of the Sync tab is built from: a text
//! prompt (optionally masked), a menu of actions, and a text panel. A flow is
//! a chain of them whose callbacks open the next one or run a job on the sync
//! worker, so a new operation needs no new mode.

use super::*;

pub(crate) type PromptFn = Box<dyn FnOnce(&mut App, String)>;
pub(crate) type MenuFn = Box<dyn FnOnce(&mut App)>;

/// What the open generic Sync modal shows and will do.
pub(crate) enum SyncUi {
    Prompt {
        title: String,
        label: String,
        masked: bool,
        value: String,
        on_submit: PromptFn,
        /// Text shown above the field, and QR frames looped under it (empty: none).
        lines: Vec<String>,
        frames: Vec<Vec<u8>>,
    },
    Menu {
        title: String,
        items: Vec<(String, MenuFn)>,
        selected: usize,
    },
    Info {
        title: String,
        lines: Vec<String>,
    },
}

impl App {
    /// Ask for a line of text. `on_submit` runs after the prompt closed, so it
    /// may open the next modal.
    pub(crate) fn sync_prompt(
        &mut self,
        title: impl Into<String>,
        label: impl Into<String>,
        masked: bool,
        on_submit: impl FnOnce(&mut App, String) + 'static,
    ) {
        self.sync_ui = Some(SyncUi::Prompt {
            title: title.into(),
            label: label.into(),
            masked,
            value: String::new(),
            on_submit: Box::new(on_submit),
            lines: Vec::new(),
            frames: Vec::new(),
        });
        self.mode = AppMode::SyncPrompt;
    }

    /// A prompt with explanatory lines and a looping QR (for the phone to scan)
    /// over its field.
    pub(crate) fn sync_prompt_qr(
        &mut self,
        title: impl Into<String>,
        lines: Vec<String>,
        frames: Vec<Vec<u8>>,
        label: impl Into<String>,
        on_submit: impl FnOnce(&mut App, String) + 'static,
    ) {
        self.sync_prompt(title, label, false, on_submit);
        if let Some(SyncUi::Prompt {
            lines: l,
            frames: f,
            ..
        }) = self.sync_ui.as_mut()
        {
            *l = lines;
            *f = frames;
        }
        self.sync.qr_frame = 0;
    }

    /// Offer a list of actions; Enter runs the highlighted one.
    pub(crate) fn sync_menu(&mut self, title: impl Into<String>, items: Vec<(String, MenuFn)>) {
        self.sync_ui = Some(SyncUi::Menu {
            title: title.into(),
            items,
            selected: 0,
        });
        self.mode = AppMode::SyncMenu;
    }

    /// Show lines of text until the user closes the panel.
    pub(crate) fn sync_info(&mut self, title: impl Into<String>, lines: Vec<String>) {
        self.sync_ui = Some(SyncUi::Info {
            title: title.into(),
            lines,
        });
        self.mode = AppMode::SyncInfo;
    }

    /// The Sync tab's admin menu: the operations that change the ring itself.
    pub fn open_sync_admin(&mut self) {
        if !self.sync.configured {
            self.host_notice = Some("sync is not set up on this profile (tab 7 says how)".into());
            return;
        }
        let items = super::sync_admin::entries();
        self.sync_menu("Manage the ring", items);
    }

    pub(crate) fn handle_key_sync_ui(&mut self, key: KeyEvent) -> Result<()> {
        let cancel = key.code == KeyCode::Esc;
        let Some(ui) = self.sync_ui.take() else {
            self.mode = AppMode::Normal;
            return Ok(());
        };
        match ui {
            SyncUi::Prompt {
                title,
                label,
                masked,
                mut value,
                on_submit,
                lines,
                frames,
            } => {
                if cancel {
                    self.mode = AppMode::Normal;
                } else if key.code == KeyCode::Enter {
                    self.mode = AppMode::Normal;
                    on_submit(self, value);
                } else {
                    match key.code {
                        KeyCode::Backspace => {
                            value.pop();
                        }
                        KeyCode::Char(c)
                            if !c.is_control()
                                && !key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            value.push(c);
                        }
                        _ => {}
                    }
                    self.sync_ui = Some(SyncUi::Prompt {
                        title,
                        label,
                        masked,
                        value,
                        on_submit,
                        lines,
                        frames,
                    });
                }
            }
            SyncUi::Menu {
                title,
                mut items,
                mut selected,
            } => {
                let last = items.len().saturating_sub(1);
                if cancel {
                    self.mode = AppMode::Normal;
                } else if key.code == KeyCode::Enter && !items.is_empty() {
                    self.mode = AppMode::Normal;
                    let (_, run) = items.remove(selected.min(last));
                    run(self);
                } else {
                    if self.is_action(KeyAction::MoveDown, &key) {
                        selected = (selected + 1).min(last);
                    } else if self.is_action(KeyAction::MoveUp, &key) {
                        selected = selected.saturating_sub(1);
                    }
                    self.sync_ui = Some(SyncUi::Menu {
                        title,
                        items,
                        selected,
                    });
                }
            }
            SyncUi::Info { title, lines } => {
                if cancel || key.code == KeyCode::Enter {
                    self.mode = AppMode::Normal;
                } else {
                    self.sync_ui = Some(SyncUi::Info { title, lines });
                }
            }
        }
        Ok(())
    }
}
