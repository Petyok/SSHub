Output demo/gifs/add-host.gif

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
Sleep 400ms
Type "203.0.113.42"
Sleep 400ms
Enter
Sleep 400ms

# Down past Password to Username
Down
Down
Enter
Sleep 400ms
Type "deploy"
Sleep 400ms
Enter
Sleep 400ms

# Label
Down
Enter
Sleep 400ms
Type "Demo VM"
Sleep 400ms
Enter
Sleep 400ms

# Name (alias)
Down
Enter
Sleep 400ms
Type "demo-vm"
Sleep 400ms
Enter
Sleep 800ms

# Save
Ctrl+S
Sleep 2s

# Find the new host, then favorite the selected row
Type "/"
Type "demo"
Sleep 2s
Escape
Sleep 800ms
Type "f"
Sleep 2.5s

Hide
Type "q"
