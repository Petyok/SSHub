Output demo/gifs/connect.gif

Set FontSize 16
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set TypingSpeed 90ms
Set PlaybackSpeed 0.75

Hide
Type "export SSHUB_CONFIG_DIR=$PWD/demo/home/.config/sshub SSHUB_DATA_DIR=$PWD/demo/home/.local/share/sshub SSHUB_SSH_CONFIG=$PWD/demo/home/ssh_config"
Enter
Type "export PATH=$PWD/demo/bin:$PWD/target/release:$PATH"
Enter
Type "bash demo/seed-demo.sh"
Enter
Type "clear && sshub"
Enter
Sleep 1s
Show

Space
Sleep 2.5s

# Connect via fuzzy palette — embedded PTY session
Type "/"
Sleep 1s
Type "google"
Sleep 1.5s
Enter
Sleep 3.5s

Type `cowsay "SSHub: secure shell x undefined behavior"`
Sleep 800ms
Enter
Sleep 3s

# Detach to dashboard; SSH keeps running (session strip in header)
Ctrl+D
Sleep 2.5s

# Re-focus the detached session from the dashboard
Ctrl+Alt+N
Sleep 2s

# Open a second session tab (host picker)
Ctrl+T
Sleep 1.5s
Type "quad"
Sleep 1.5s
Enter
Sleep 3.5s

# Switch between session tabs
Ctrl+Shift+A
Sleep 2s
Ctrl+Shift+D
Sleep 2s

Ctrl+W
Sleep 1.5s

# Close the active session tab from the dashboard
Ctrl+W
Sleep 1.5s

Hide
Type "q"
