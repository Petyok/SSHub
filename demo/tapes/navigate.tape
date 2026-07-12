Output demo/build/navigate.mp4

Set FontSize 16
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set TypingSpeed 40ms
Set PlaybackSpeed 1
Set Framerate 24

Hide
# Boot with the startup animation disabled so we land straight on the app.
Type "rm -rf /tmp/sshub-nav && cp -r $PWD/demo/home/.config/sshub /tmp/sshub-nav && sed -i 's/disable_animation = false/disable_animation = true/' /tmp/sshub-nav/config.toml"
Enter
Wait
Type "export SSHUB_VERSION_LABEL= SSHUB_CONFIG_DIR=/tmp/sshub-nav SSHUB_DATA_DIR=$PWD/demo/home/.local/share/sshub SSHUB_SSH_CONFIG=$PWD/demo/home/ssh_config PATH=$PWD/target/release:$PATH"
Enter
Wait
Type "bash demo/seed-demo.sh"
Enter
Wait@120s
Type "clear && sshub"
Enter
Sleep 2s
Show

Sleep 1.5s

# Walk into the nested groups.
Down
Sleep 700ms
Down
Sleep 700ms
Down
Sleep 700ms
Down
Sleep 1s

# Collapse the group, then expand it again.
Space
Sleep 1.6s
Space
Sleep 1.6s

# Fuzzy host palette.
Type "/"
Sleep 800ms
Type@80ms "quad"
Sleep 1.6s
Escape
Sleep 800ms

# Group manager overlay (nested groups).
Shift+G
Sleep 1.6s
Down
Sleep 800ms
Down
Sleep 800ms
Escape
Sleep 1s

# Multi-tag filter: # opens the picker, Space toggles, Enter applies.
Type "#"
Sleep 1.2s
Down
Sleep 700ms
Space
Sleep 800ms
Down
Sleep 700ms
Space
Sleep 1s
Enter

# Hold on the filtered list.
Sleep 3.5s

Hide
Type "q"
