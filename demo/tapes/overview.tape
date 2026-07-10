Output demo/gifs/overview.gif

Set FontSize 16
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set TypingSpeed 90ms
Set PlaybackSpeed 0.75

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
Down
Sleep 1.1s
Down
Sleep 1.1s
Down
Sleep 1.1s
Up
Sleep 1.5s
Up
Sleep 1.5s

# Collapse the group, then expand it again.
Space
Sleep 2s
Space
Sleep 2s

# Multi-tag filter: # opens the picker, Space toggles, Enter applies.
Type "#"
Sleep 1.8s
Down
Sleep 1s
Space
Sleep 1s
Space
Sleep 1.8s
Down
Sleep 1s
Space
Sleep 1.8s
Down
Sleep 1.8s
Down
Sleep 1s
Space
Sleep 2s
Enter
Sleep 2.5s
Escape
Sleep 1.5s

# Group manager overlay (nested groups).
Shift+G
Sleep 1.8s
Down
Sleep 1.8s
Down
Sleep 1.8s
Escape
Sleep 1.8s

# Fuzzy host palette.
Type "/"
Sleep 1.8s
Type "quad"
Sleep 1.8s
Escape
Sleep 1.8s

# Tour the tabs: sftp, tunnels, identities, audit, back to hosts.
Type "2"
Sleep 1.8s
Type "3"
Sleep 1.8s
Type "4"
Sleep 1.8s
Type "5"
Sleep 1.8s
Type "1"
Sleep 2s

# Settings overlay: opaque background, OS logos, and other toggles (Ctrl+H).
Ctrl+H
Sleep 1s
Down
Sleep 900ms
Down
Sleep 900ms
Escape
Sleep 1.5s

# Help screen.
Ctrl+K
Sleep 1s
Down
Sleep 800ms
Down
Sleep 800ms
Down
Sleep 800ms
Down
Sleep 800ms
Down
Sleep 800ms
Down
Sleep 800ms
Escape
Sleep 1.5s

Type "="
Sleep 800ms
Type "="
Sleep 800ms
Type "="
Sleep 800ms
Type "-"
Sleep 800ms
Type "-"
Sleep 4s

Hide
Type "q"
