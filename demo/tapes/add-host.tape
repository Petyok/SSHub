Output demo/build/add-host.mp4

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
Type "rm -rf /tmp/sshub-add && cp -r $PWD/demo/home/.config/sshub /tmp/sshub-add && sed -i 's/disable_animation = false/disable_animation = true/' /tmp/sshub-add/config.toml"
Enter
Wait
Type "export SSHUB_VERSION_LABEL= SSHUB_CONFIG_DIR=/tmp/sshub-add SSHUB_DATA_DIR=$PWD/demo/home/.local/share/sshub SSHUB_SSH_CONFIG=$PWD/demo/home/ssh_config PATH=$PWD/demo/bin:$PWD/target/demo/release:$PATH"
Enter
Wait
Type "bash demo/seed-demo.sh"
Enter
Wait@120s
Type "clear && sshub"
Enter
Sleep 2s
Show

Sleep 1s

# Open the add-host form.
Type "a"
Sleep 1s

# Address (first field).
Type "172.217.21.238"
Sleep 400ms

# Password.
Down
Sleep 400ms
Type "BlahblahblahPASSWORD!!!"
Sleep 400ms

# Username.
Down
Sleep 400ms
Type "pizza-lover-9000"
Sleep 400ms

# Label.
Down
Sleep 400ms
Type "Demo VM"
Sleep 400ms

# Name (alias).
Down
Sleep 400ms
Type "demo-vm-1"
Sleep 400ms

# Port (default 22).
Down
Sleep 400ms

# Group — open the multi-select checkbox picker, tick a couple of groups
# (Space toggles, Enter closes). A host can belong to several groups.
Down
Enter
Sleep 700ms
Down
Sleep 400ms
Space
Sleep 500ms
Down
Sleep 400ms
Down
Sleep 400ms
Space
Sleep 600ms
Enter
Sleep 500ms

# Identity (keep default).
Down
Sleep 400ms

# Tags.
Down
Sleep 400ms
Type "demo,pizza"
Sleep 500ms

# Save, then find the new host with the fuzzy palette.
Ctrl+S
Sleep 1.5s

Type "/"
Sleep 800ms
Type "Demo"
Sleep 3s

Hide
Type "q"
