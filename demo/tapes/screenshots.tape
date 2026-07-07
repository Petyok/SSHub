# Static screenshots for the README. Boots the demo fixtures and snaps each
# key screen with VHS's `Screenshot` command. The scratch gif is tiny (short
# tape) and can be deleted after — only the PNGs matter.
Output demo/screenshots/_scratch.gif

Set FontSize 16
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set TypingSpeed 60ms

Hide
# Screenshot config = the demo config with the startup animation disabled, so
# the app opens straight on the dashboard (no racing the splash timing).
Type "rm -rf /tmp/sshub-shot && cp -r $PWD/demo/home/.config/sshub /tmp/sshub-shot && sed -i 's/disable_animation = false/disable_animation = true/' /tmp/sshub-shot/config.toml"
Enter
Type "export SSHUB_CONFIG_DIR=/tmp/sshub-shot SSHUB_DATA_DIR=$PWD/demo/home/.local/share/sshub SSHUB_SSH_CONFIG=$PWD/demo/home/ssh_config PATH=$PWD/target/release:$PATH"
Enter
Type "bash demo/seed-demo.sh"
Enter
Type "clear && sshub"
Enter
Sleep 2.5s
Show

# 1 — Hosts dashboard (groups + live dashboard panels)
Sleep 800ms
Screenshot demo/screenshots/hosts.png

# 2 — Fuzzy host palette
Type "/"
Sleep 800ms
Type "quad"
Sleep 1s
Screenshot demo/screenshots/palette.png
Escape
Sleep 500ms

# 3 — Multi-tag filter picker
Type "#"
Sleep 1s
Screenshot demo/screenshots/tags.png
Escape
Sleep 500ms

# 4 — Add-host form
Type "a"
Sleep 1s
Screenshot demo/screenshots/add-host.png
Escape
Sleep 500ms

# 5 — Keybinding editor (Ctrl+K)
Ctrl+K
Sleep 1s
Screenshot demo/screenshots/keybindings.png
Escape
Sleep 500ms

# 6 — Help overlay
Type "?"
Sleep 1s
Screenshot demo/screenshots/help.png
Escape
Sleep 500ms

Hide
Type "q"
