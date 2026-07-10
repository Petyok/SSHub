Output demo/gifs/add-host.gif

Set FontSize 16
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set TypingSpeed 90ms
Set PlaybackSpeed 0.75

Hide
Type "export SSHUB_CONFIG_DIR=$PWD/demo/home/.config/sshub SSHUB_DATA_DIR=$PWD/demo/home/.local/share/sshub SSHUB_SSH_CONFIG=$PWD/demo/home/ssh_config PATH=$PWD/target/release:$PATH"
Enter
Type "export PATH=$PWD/demo/bin:$PWD/target/release:$PATH"
Enter
Type "bash demo/seed-demo.sh"
Enter
Type "clear"
Enter
Sleep 2s
Space
Type "sshub"
Enter
Sleep 1s
Space
Show

Sleep 1.5s

# Open the add-host form
Type "a"
Sleep 1.5s

# Address (first field)
Sleep 800ms
Type "172.217.21.238"
Sleep 800ms

# Password
Down
Sleep 800ms
Type "BlahblahblahPASSWORD!!!"
Sleep 800ms

# Username
Down
Sleep 800ms
Type "pizza-lover-9000"
Sleep 800ms

# Label
Down
Sleep 800ms
Type "Demo VM"
Sleep 800ms

# Name (alias)
Down
Sleep 800ms
Type "demo-vm-1"
Sleep 800ms

# Port (default 22)
Down
Sleep 800ms

# Group — open the multi-select checkbox picker, tick a couple of groups
# (Space toggles, Enter closes). A host can now belong to several groups.
Down
Enter
Sleep 1s
Down
Sleep 600ms
Space
Sleep 800ms
Down
Sleep 600ms
Down
Sleep 600ms
Space
Sleep 900ms
Enter
Sleep 800ms

# Identity (keep default)
Down
Sleep 800ms

# Tags
Down
Sleep 800ms
Type "demo,pizza"
Sleep 800ms

# Save
Ctrl+S
Sleep 2s

Type "/"
Sleep 2s
Type "Demo"
Sleep 4s

Hide
Type "q"
