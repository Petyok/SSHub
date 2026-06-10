Output demo/gifs/overview.gif

Set FontSize 16
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set TypingSpeed 60ms

Hide
Type "export SSHUB_CONFIG_DIR=demo/home/.config/sshub SSHUB_DATA_DIR=demo/home/.local/share/sshub SSHUB_SSH_CONFIG=demo/home/ssh_config PATH=$PWD/target/release:$PATH"
Enter
Type "rm -f demo/home/.local/share/sshub/launcher.db*"
Enter
Type "clear && sshub"
Enter
Sleep 1s
Show

# Intro animation, then skip into the app
Sleep 13s
Space
Sleep 2s

# Browse hosts
Type "j"
Sleep 1s
Type "j"
Sleep 1s
Type "j"
Sleep 1s
Type "k"
Sleep 1s
Type "k"
Sleep 1s

# Toggle detail panel
Tab
Sleep 2s
Tab
Sleep 1s

# Fuzzy search
Type "/"
Sleep 1s
Type "db"
Sleep 2s
Escape
Sleep 1s

# Tour the tabs
Type "2"
Sleep 3s
Type "3"
Sleep 3s
Type "4"
Sleep 3s
Type "1"
Sleep 2s

# Help screen
Type "?"
Sleep 3s
Escape
Sleep 2s

Hide
Type "q"
