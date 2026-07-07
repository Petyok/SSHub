Output demo/gifs/overview.gif

Set FontSize 16
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set TypingSpeed 60ms

Hide
Type "export SSHUB_CONFIG_DIR=$PWD/demo/home/.config/sshub SSHUB_DATA_DIR=$PWD/demo/home/.local/share/sshub SSHUB_SSH_CONFIG=$PWD/demo/home/ssh_config PATH=$PWD/target/release:$PATH"
Enter
Type "bash demo/seed-demo.sh"
Enter
Type "clear && sshub"
Enter
Sleep 1s
Show

# Splash animation (collab tagline + random quip), then skip
Sleep 13s
Space
Sleep 2s

# Browse nested groups (Production → Web / Databases)
Type "j"
Sleep 800ms
Type "j"
Sleep 800ms
Type "j"
Sleep 800ms
Type "k"
Sleep 1s

# Collapse / expand the Production subtree
Space
Sleep 1.5s
Space
Sleep 1s

# Detail panel + reachability stats
Tab
Sleep 2.5s
Tab
Sleep 1s

# Multi-tag filter (# opens picker; Space toggles; Enter applies)
Type "#"
Sleep 1s
Type "j"
Sleep 600ms
Type "j"
Sleep 600ms
Space
Sleep 1.5s
Enter
Sleep 2s
Escape
Sleep 1s

# Group manager overlay (nested groups)
Shift+G
Sleep 2s
Type "j"
Sleep 1s
Type "j"
Sleep 1s
Escape
Sleep 1s

# Configurable keybindings editor
Ctrl+K
Sleep 2.5s
Escape
Sleep 1s

# Fuzzy host palette
Type "/"
Sleep 1s
Type "db"
Sleep 2s
Escape
Sleep 1s

# Tour tabs: tunnels, keys, audit, back to hosts
Type "2"
Sleep 2.5s
Type "3"
Sleep 2.5s
Type "4"
Sleep 2.5s
Type "1"
Sleep 1.5s

# Help — session keys, tags, groups
Type "?"
Sleep 4s
Escape
Sleep 1s

Hide
Type "q"
