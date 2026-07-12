Output demo/build/add-host.mp4

Set FontSize 16
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set TypingSpeed 40ms
Set PlaybackSpeed 1
Set Framerate 24

Hide
# Boot with the startup animation disabled so we land straight on the app.
Type "rm -rf /tmp/sshub-add && cp -r $PWD/demo/home/.config/sshub /tmp/sshub-add && sed -i 's/disable_animation = false/disable_animation = true/' /tmp/sshub-add/config.toml"
Enter
Wait
Type "export SSHUB_VERSION_LABEL= SSHUB_CONFIG_DIR=/tmp/sshub-add SSHUB_DATA_DIR=$PWD/demo/home/.local/share/sshub SSHUB_SSH_CONFIG=$PWD/demo/home/ssh_config PATH=$PWD/demo/bin:$PWD/target/release:$PATH"
Enter
Wait
Type "bash demo/seed-demo.sh"
Enter
Wait@120s
Type "clear && sshub"
Enter
Sleep 2s
Show

Sleep 1s

# Open the add-host form.
Type "a"
Sleep 1s

# Address (first field).
Type "172.217.21.238"
Sleep 400ms

# Password.
Down
Sleep 400ms
Type "BlahblahblahPASSWORD!!!"
Sleep 400ms

# Username.
Down
Sleep 400ms
Type "pizza-lover-9000"
Sleep 400ms

# Label.
Down
Sleep 400ms
Type "Demo VM"
Sleep 400ms

# Name (alias).
Down
Sleep 400ms
Type "demo-vm-1"
Sleep 400ms

# Port (default 22).
Down
Sleep 400ms

# Group — open the multi-select checkbox picker, tick a couple of groups
# (Space toggles, Enter closes). A host can belong to several groups.
Down
Enter
Sleep 700ms
Down
Sleep 400ms
Space
Sleep 500ms
Down
Sleep 400ms
Down
Sleep 400ms
Space
Sleep 600ms
Enter
Sleep 500ms

# Identity (keep default).
Down
Sleep 400ms

# Tags.
Down
Sleep 400ms
Type "demo,pizza"
Sleep 500ms

# Save, then find the new host with the fuzzy palette.
Ctrl+S
Sleep 1.5s

Type "/"
Sleep 800ms
Type "Demo"
Sleep 3s

Hide
Type "q"
