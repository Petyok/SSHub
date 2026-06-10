Output demo/gifs/connect.gif

Set FontSize 16
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set TypingSpeed 60ms

Hide
Type "export SSHUB_CONFIG_DIR=demo/home/.config/sshub SSHUB_DATA_DIR=demo/home/.local/share/sshub SSHUB_SSH_CONFIG=demo/home/ssh_config"
Enter
Type "export PATH=$PWD/demo/bin:$PWD/target/release:$PATH"
Enter
Type "rm -f demo/home/.local/share/sshub/launcher.db*"
Enter
Type "clear && sshub"
Enter
Sleep 1s
Space
Sleep 1s
Show

Sleep 1.5s

# Pick a host and connect (Enter spawns the demo ssh on a PTY)
Type "/"
Sleep 1.5s
Type "web-prod-01"
Sleep 1.5s
Enter
Sleep 1s

# The scripted connect animation, then the (faked) remote shell
Sleep 3s

# Run something silly on the "remote"
Type `cowsay "SSHub: 100% vibe-coded slop, made with Claude Opus 4.8"`
Sleep 800ms
Enter
Sleep 3s

# Back out of the session and quit
Ctrl+D
Sleep 1.5s

Hide
Type "q"
