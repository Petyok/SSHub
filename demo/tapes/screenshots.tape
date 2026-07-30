# Static screenshots for the README. Boots the demo fixtures and snaps each
# key screen with VHS's `Screenshot` command. The scratch mp4 lands in the
# gitignored demo/build/ — only the PNGs matter.
Output demo/build/screenshots.mp4

Set FontSize 16
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set TypingSpeed 60ms
# Deliberately low, and lower than it looks like it should be. VHS writes the
# container at a FIXED 25 fps while `Set Framerate` only sets how fast it
# screenshots the terminal. When it sustains that rate it duplicates frames up to
# 25 and the timeline is correct; when a screenshot costs more than the budget
# (which is what happens on a full-screen repaint) frames are simply missing and
# the take plays SPED UP. Measured on 10s of continuous full-screen motion:
#
#   Framerate 60 -> 6.04s (0.60x)   Framerate 20 -> 9.12s (0.91x)
#   Framerate 25 -> 8.84s (0.88x)   Framerate 12 -> 9.84s (0.98x)
#
# So raising this makes motion worse, not smoother. 12 keeps real time, and the
# demo build stretches the animations (see demo/record.sh) so a slide still gets
# ~12 distinct frames.
Set Framerate 12

Hide
# Screenshot config = the demo config with the startup animation disabled, so
# the app opens straight on the dashboard (no racing the splash timing).
Type "rm -rf /tmp/sshub-shot && cp -r $PWD/demo/home/.config/sshub /tmp/sshub-shot && sed -i 's/disable_animation = false/disable_animation = true/' /tmp/sshub-shot/config.toml"
Enter
Wait
Type "export SSHUB_VERSION_LABEL= SSHUB_CONFIG_DIR=/tmp/sshub-shot SSHUB_DATA_DIR=$PWD/demo/home/.local/share/sshub SSHUB_SSH_CONFIG=$PWD/demo/home/ssh_config PATH=$PWD/target/demo/release:$PATH"
Enter
Wait
Type "bash demo/seed-demo.sh"
Enter
Wait@120s
Type "clear && sshub"
Enter
Sleep 2.5s
Show

# 1 — Hosts dashboard: select a host so the card renders its OS logo, fact
# sheet, and per-host latency panel.
Down
Sleep 2s
Screenshot demo/screenshots/hosts.png

# 2 — Fuzzy host palette
Type "/"
Sleep 800ms
Type "quad"
Sleep 2s
Screenshot demo/screenshots/palette.png
Escape
Sleep 500ms

# 3 — Multi-tag filter picker
Type "#"
Sleep 2s
Screenshot demo/screenshots/tags.png
Escape
Sleep 500ms

# 4 — Add-host form
Type "a"
Sleep 2s
Screenshot demo/screenshots/add-host.png
Escape
Sleep 500ms

# 5 — Keybinding editor (Ctrl+K)
Ctrl+K
Sleep 2s
Screenshot demo/screenshots/keybindings.png
Escape
Sleep 500ms

# 6 — Help overlay
Type "?"
Sleep 2s
Screenshot demo/screenshots/help.png
Escape
Sleep 500ms

# 7 — Settings overlay (Ctrl+H) — toggles incl. opaque background + OS logos
Ctrl+H
Sleep 2s
Screenshot demo/screenshots/settings.png
Escape
Sleep 500ms

Hide
Type "q"
