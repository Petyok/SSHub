Output demo/gifs/sftp.gif

Set FontSize 16
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set TypingSpeed 90ms
Set PlaybackSpeed 0.75

Hide
# Boot with the startup animation disabled so we land straight on the app, and
# spin up the throwaway loopback SFTP server the demo host points at.
Type "rm -rf /tmp/sshub-sftp && cp -r $PWD/demo/home/.config/sshub /tmp/sshub-sftp && sed -i 's/disable_animation = false/disable_animation = true/' /tmp/sshub-sftp/config.toml"
Enter
Type "bash demo/sftp-server.sh start"
Enter
Type "export SSHUB_CONFIG_DIR=/tmp/sshub-sftp SSHUB_DATA_DIR=$PWD/demo/home/.local/share/sshub SSHUB_SSH_CONFIG=$PWD/demo/home/ssh_config PATH=$PWD/demo/bin:$PWD/target/release:$PATH"
Enter
Type "bash demo/seed-demo.sh"
Enter
# Launch from the curated local folder so the right (local) pane is tidy.
Type "cd demo/home/sftp-local && clear && sshub"
Enter
Sleep 2s
Show

# Open the SFTP tab and connect to the demo server.
Type "2"
Sleep 1.5s
Type "/"
Sleep 600ms
Type "SFTP demo"
Sleep 1s
Enter
Sleep 3s

# Browse the remote tree: descend into a directory and back out.
Down
Sleep 800ms
Enter
Sleep 1.5s
Backspace
Sleep 1.5s

# Stage a download (remote file -> local pane).
Down
Sleep 600ms
Down
Sleep 600ms
Left
Sleep 1s
Screenshot demo/screenshots/sftp.png

# Switch focus to the local pane and stage an upload (local file -> remote).
Tab
Sleep 900ms
Down
Sleep 600ms
Right
Sleep 1.2s

# Run the queued transfers with a progress bar.
Type "c"
Sleep 3s
Sleep 1.5s

Hide
Type "q"
Enter
Type "bash ../../sftp-server.sh stop"
Enter
