Output demo/build/connect.mp4

Set FontSize 16
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set TypingSpeed 40ms
Set PlaybackSpeed 1
Set Framerate 24

Hide
# Boot with the startup animation disabled so we land straight on the app.
Type "rm -rf /tmp/sshub-connect && cp -r $PWD/demo/home/.config/sshub /tmp/sshub-connect && sed -i 's/disable_animation = false/disable_animation = true/' /tmp/sshub-connect/config.toml"
Enter
Wait
Type "export SSHUB_VERSION_LABEL= SSHUB_CONFIG_DIR=/tmp/sshub-connect SSHUB_DATA_DIR=$PWD/demo/home/.local/share/sshub SSHUB_SSH_CONFIG=$PWD/demo/home/ssh_config PATH=$PWD/demo/bin:$PWD/target/release:$PATH"
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

# Connect via the fuzzy palette — the session runs in an embedded PTY.
Type "/"
Sleep 500ms
Type "google"
Sleep 800ms
Enter
Sleep 3s

Type `cowsay "SSHub: secure shell x undefined behavior"`
Sleep 400ms
Enter
Sleep 2s

# Detach to the dashboard; SSH keeps running (session strip in the header).
Ctrl+D
Sleep 2s

# Re-focus the detached session from the dashboard.
Ctrl+Alt+N
Sleep 1.5s

# Open a second session tab (host picker).
Ctrl+T
Sleep 1s
Type "quad"
Sleep 800ms
Enter
Sleep 2.5s

# Switch between session tabs, then detach.
Ctrl+A
Sleep 1.5s
Ctrl+D
Sleep 1.5s

# Close both session tabs from the dashboard.
Ctrl+W
Sleep 1s
Ctrl+W

# Hold on the dashboard.
Sleep 2.5s

Hide
Type "q"
