Output demo/gifs/overview.gif

Set FontSize 16
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set TypingSpeed 100ms

Hide
Type "export SSHUB_CONFIG_DIR=$PWD/demo/home/.config/sshub SSHUB_DATA_DIR=$PWD/demo/home/.local/share/sshub SSHUB_SSH_CONFIG=$PWD/demo/home/ssh_config PATH=$PWD/target/release:$PATH"
Enter
Type "bash demo/seed-demo.sh"
Enter
Type "clear && sshub"
Enter
Sleep 1s
Show

# Splash animation (collab tagline + random quip), then skip into the app.
Sleep 13s
Enter
Sleep 2.5s

# Browse the host tree (nested groups).
Type "j"
Sleep 1.1s
Type "j"
Sleep 1.1s
Type "j"
Sleep 1.1s
Type "k"
Sleep 1.5s

# Collapse the group, then expand it again.
Space
Sleep 2s
Space
Sleep 2s

# Detail panel + reachability stats.
Tab
Sleep 3s
Tab
Sleep 1.5s

# Multi-tag filter: # opens the picker, Space toggles, Enter applies.
Type "#"
Sleep 1.8s
Type "j"
Sleep 1s
Space
Sleep 2s
Enter
Sleep 2.5s
Escape
Sleep 1.5s

# Group manager overlay (nested groups).
Shift+G
Sleep 3s
Type "j"
Sleep 1.5s
Type "j"
Sleep 1.5s
Escape
Sleep 1.5s

# Fuzzy host palette.
Type "/"
Sleep 1.5s
Type "db"
Sleep 2.5s
Escape
Sleep 1.5s

# Tour the tabs: tunnels, keys, audit, back to hosts.
Type "2"
Sleep 3s
Type "3"
Sleep 3s
Type "4"
Sleep 3s
Type "1"
Sleep 2s

# Help screen.
Type "?"
Sleep 4s
Escape
Sleep 1.5s

Hide
Type "q"
