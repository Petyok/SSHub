#!/bin/sh
# Create the SSH host key on first start. It lives on the data volume, so the
# hub keeps the same identity across container restarts and re-creation.
set -eu

keydir=/home/hub/.local/share/sshub/ssh-host-keys
install -d -m 0700 "$keydir"
if [ ! -f "$keydir/ssh_host_ed25519_key" ]; then
    ssh-keygen -q -t ed25519 -N '' -f "$keydir/ssh_host_ed25519_key"
fi

# The hub has nobody at its screen: it enrolls as an UNATTENDED mirror (never votes, never a required
# checkpoint signer). A config.toml mounted by the operator must carry `[sync] unattended = true` too.
cfg=/home/hub/.config/sshub/config.toml
if [ ! -f "$cfg" ]; then
    printf '[sync]\nunattended = true\n' > "$cfg"
fi

# -D: stay in the foreground so the container lives as long as sshd does.
exec /usr/sbin/sshd -D -e -f /etc/ssh/sshd_config
