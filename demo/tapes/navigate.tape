Output demo/build/navigate.mp4

Set FontSize 16
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set TypingSpeed 40ms
Set PlaybackSpeed 1
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
# Boot with the startup animation disabled so we land straight on the app.
Type "rm -rf /tmp/sshub-nav && cp -r $PWD/demo/home/.config/sshub /tmp/sshub-nav && sed -i 's/disable_animation = false/disable_animation = true/' /tmp/sshub-nav/config.toml"
Enter
Wait
Type "export SSHUB_VERSION_LABEL= SSHUB_CONFIG_DIR=/tmp/sshub-nav SSHUB_DATA_DIR=$PWD/demo/home/.local/share/sshub SSHUB_SSH_CONFIG=$PWD/demo/home/ssh_config PATH=$PWD/target/demo/release:$PATH"
Enter
Wait
Type "bash demo/seed-demo.sh"
Enter
Wait@120s
Type "clear && sshub"
Enter
Sleep 2s
Show

Sleep 1.5s

# Walk into the nested groups.
Down
Sleep 700ms
Down
Sleep 700ms
Down
Sleep 700ms
Down
Sleep 1s

# Collapse the group, then expand it again.
Space
Sleep 1.6s
Space
Sleep 1.6s

# Fuzzy host palette.
Type "/"
Sleep 800ms
Type@80ms "quad"
Sleep 1.6s
Escape
Sleep 800ms

# Group manager overlay (nested groups).
Shift+G
Sleep 1.6s
Down
Sleep 800ms
Down
Sleep 800ms
Escape
Sleep 1s

# Multi-tag filter: # opens the picker, Space toggles, Enter applies.
Type "#"
Sleep 1.2s
Down
Sleep 700ms
Space
Sleep 800ms
Down
Sleep 700ms
Space
Sleep 1s
Enter

# Hold on the filtered list.
Sleep 3.5s

Hide
Type "q"
