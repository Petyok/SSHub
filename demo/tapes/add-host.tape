Output demo/gifs/add-host.gif

Set FontSize 16
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set TypingSpeed 75ms

Hide
Type "export SSHUB_CONFIG_DIR=$PWD/demo/home/.config/sshub SSHUB_DATA_DIR=$PWD/demo/home/.local/share/sshub SSHUB_SSH_CONFIG=$PWD/demo/home/ssh_config PATH=$PWD/target/release:$PATH"
Enter
Type "bash demo/seed-demo.sh"
Enter
Type "clear && sshub"
Enter
Sleep 2s
Space
Sleep 1s
Show

Sleep 1.5s

# Open the add-host form
Type "a"
Sleep 1.5s

# Address (first field)
Enter
Sleep 800ms
Type "203.0.113.42"
Sleep 800ms
Enter
Sleep 800ms

# Password
Down
Enter
Sleep 800ms
Type "BlahblahblahPASSWORD!!!"
Sleep 800ms
Enter

# Username
Down
Enter
Sleep 800ms
Type "pizza-lover-9000"
Sleep 800ms
Enter

# Label
Down
Enter
Sleep 800ms
Type "Demo VM"
Sleep 800ms
Enter
Sleep 800ms

# Name (alias)
Down
Enter
Sleep 800ms
Type "demo-vm-1"
Sleep 800ms
Enter
Sleep 800ms

# Port (default 22)
Down
Enter
Sleep 800ms
Enter
Sleep 800ms

# Group — pick Staging from seeded groups
Down
Enter
Sleep 800ms
Type "j"
Sleep 800ms
Type "j"
Sleep 800ms
Enter
Sleep 800ms

# Identity (keep default)
Down
Enter
Sleep 800ms
Enter
Sleep 800ms

# Tags
Down
Enter
Sleep 800ms
Type "demo, pizza"
Sleep 800ms
Enter
Sleep 800ms

# Save
Ctrl+S
Sleep 2s

# Find the new host and favorite it
Type "/"
Sleep 1.5s
Type "demo"
Sleep 2s
Escape
Sleep 800ms
Type "f"
Sleep 2.5s

Hide
Type "q"
