Output demo/gifs/overview.gif

Set FontSize 16
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set TypingSpeed 60ms

Hide
Type "export SSHUB_CONFIG_DIR=demo/home/.config/sshub SSHUB_DATA_DIR=demo/home/.local/share/sshub SSHUB_SSH_CONFIG=demo/home/ssh_config PATH=$PWD/target/release:$PATH"
Enter
Type "clear && sshub"
Enter
Sleep 1s
Show

# Intro animation, then skip into the app
Sleep 4s
Space
Sleep 2s

# Browse hosts
Type "jjj"
Sleep 1s
Type "kk"
Sleep 1s

# Toggle detail panel
Tab
Sleep 2s
Tab
Sleep 1s

# Fuzzy search
Type "/"
Sleep 500ms
Type "db"
Sleep 2s
Escape
Sleep 1s

# Tour the tabs
Type "2"
Sleep 2s
Type "3"
Sleep 2s
Type "4"
Sleep 2s
Type "1"
Sleep 1.5s

# Help screen
Type "?"
Sleep 3s
Escape
Sleep 1.5s

Hide
Type "q"
