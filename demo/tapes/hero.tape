Output demo/build/hero.mp4

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
# Wait for the prompt after every setup command — typing into a still-running
# command (e.g. seed-demo.sh compiling its cargo example) derails the tape.
Type "export SSHUB_VERSION_LABEL= SSHUB_CONFIG_DIR=$PWD/demo/home/.config/sshub SSHUB_DATA_DIR=$PWD/demo/home/.local/share/sshub SSHUB_SSH_CONFIG=$PWD/demo/home/ssh_config PATH=$PWD/demo/bin:$PWD/target/demo/release:$PATH"
Enter
Wait
Type "bash demo/seed-demo.sh"
Enter
Wait@120s
Type "clear && sshub"
Enter
# The ~10s intro animation plays off-camera; open on its settled final frame
# (logo + tagline + blinking "press Enter to continue").
Sleep 11.5s
Show

Sleep 2.5s
Enter
Sleep 1.2s

# Walk the host tree — the card follows with OS logo, facts, and latency.
Down
Sleep 700ms
Down
Sleep 700ms
Down
Sleep 1s

# Fuzzy palette straight into an embedded PTY session.
Type "/"
# Long enough for the popup slide to land (780ms in a demo-motion build).
Sleep 1s
Type "google"
Sleep 900ms
Enter
Sleep 3s

Type "uptime"
Sleep 400ms
Enter
Sleep 1.8s

# Detach — SSH keeps running (session strip in the header).
Ctrl+D
Sleep 3s

Hide
Ctrl+W
Type "q"
