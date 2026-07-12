Output demo/build/hero.mp4

Set FontSize 16
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set TypingSpeed 40ms
Set PlaybackSpeed 1
Set Framerate 24

Hide
# Wait for the prompt after every setup command — typing into a still-running
# command (e.g. seed-demo.sh compiling its cargo example) derails the tape.
Type "export SSHUB_VERSION_LABEL= SSHUB_CONFIG_DIR=$PWD/demo/home/.config/sshub SSHUB_DATA_DIR=$PWD/demo/home/.local/share/sshub SSHUB_SSH_CONFIG=$PWD/demo/home/ssh_config PATH=$PWD/demo/bin:$PWD/target/release:$PATH"
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
Sleep 600ms
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
