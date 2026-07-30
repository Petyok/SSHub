Output demo/build/sftp.mp4

Set FontSize 16
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set TypingSpeed 90ms
Set PlaybackSpeed 1
# Deliberately low, and lower than it looks like it should be. VHS writes the
# container at a FIXED 25 fps while `Set Framerate` only sets how fast it
# screenshots the terminal. When it sustains that rate it duplicates frames up to
# 25 and the timeline is correct; when a screenshot costs more than the budget
# (which is what happens on a full-screen repaint) frames are simply missing and
# the take plays SPED UP. Measured on 10s of continuous full-screen motion:
#
#   Framerate 60 -> 6.04s (0.60x)   Framerate 20 -> 9.12s (0.91x)
#   Framerate 25 -> 8.84s (0.88x)   Framerate 12 -> 9.84s (0.98x)
#
# So raising this makes motion worse, not smoother. 12 keeps real time, and the
# demo build stretches the animations (see demo/record.sh) so a slide still gets
# ~12 distinct frames.
Set Framerate 12

Hide
# Boot with the startup animation disabled so we land straight on the app, and
# spin up the throwaway loopback SFTP server the demo host points at.
Type "rm -rf /tmp/sshub-sftp && cp -r $PWD/demo/home/.config/sshub /tmp/sshub-sftp && sed -i 's/disable_animation = false/disable_animation = true/' /tmp/sshub-sftp/config.toml"
Enter
Wait
Type "bash demo/sftp-server.sh start"
Enter
Wait@60s
Type "export SSHUB_VERSION_LABEL= SSHUB_CONFIG_DIR=/tmp/sshub-sftp SSHUB_DATA_DIR=$PWD/demo/home/.local/share/sshub SSHUB_SSH_CONFIG=$PWD/demo/home/ssh_config PATH=$PWD/demo/bin:$PWD/target/demo/release:$PATH"
Enter
Wait
Type "bash demo/seed-demo.sh"
Enter
Wait@120s
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
Sleep 2s
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

# --- File operations on the remote pane ---
# Focus remote, then use / search to target a specific entry for each op.
Tab
Sleep 700ms

# chmod: filter to README.md, change its octal permissions.
Type "/"
Type "README"
Sleep 800ms
Enter
Sleep 700ms
Type "M"
Sleep 1s
Backspace
Backspace
Backspace
Type "600"
Sleep 800ms
Enter
Sleep 1.5s
Type "/"
Escape
Sleep 600ms

# New folder.
Type "n"
Sleep 800ms
Type "backups"
Sleep 800ms
Enter
Sleep 1.5s

# Delete it (with confirmation).
Type "/"
Type "backups"
Sleep 800ms
Enter
Sleep 500ms
Type "d"
Sleep 1.2s
Type "y"
Sleep 1.5s
Type "/"
Escape
Sleep 800ms

Hide
Type "q"
Enter
Type "bash ../../sftp-server.sh stop"
Enter
