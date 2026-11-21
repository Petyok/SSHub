# sshub hub

A hub is a mirror of your sshub state with a permanent address. It runs sshd and
the sshub binary in a container. It has no accounts of its own, holds no
authoritative copy, and does not vote. Your devices keep working without it.

## What it does and does not do

- **Does**: runs as an ordinary sshub ring device: an unattended mirror behind
  this sshd. Terminals reach it with `ssh hub sshub sync-serve`. Key-based SSH
  logins for the `hub` account are accepted. When it receives recovery ops, it
  announces them through the `[sync.notify]` mail command, using send-only SMTP
  credentials that live on the hub, not on your devices.
- **Does not**: add a `hub-serve` verb or run a daemon of its own. It holds no
  authoritative copy and does not vote. Your devices keep working without it.
- **Does not** hold your secrets in the image. Keys, SMTP settings and sshub
  config are mounted from the host.

## Setup

1. Create `authorized_keys` with the public keys allowed to log in (one per
   line). Make it owned by the hub account, which is uid 10001 in the container:

   ```sh
   cp ~/.ssh/id_ed25519.pub authorized_keys
   sudo chown 10001:10001 authorized_keys
   chmod 0600 authorized_keys
   ```

   sshd refuses keys if the file is owned by someone else (StrictModes).

2. Optional, notifications. Create `msmtprc` with a send-only account for your
   SMTP relay, and `chmod 0600 msmtprc`. Uncomment its line in
   `docker-compose.yml`. Example (replace the host and sender):

   ```text
   account default
   host smtp.example.com
   port 587
   tls on
   tls_starttls on
   auth on
   user sshub-notify@example.com
   password <app password>
   from sshub-notify@example.com
   ```

   Then point sshub at it in `config.toml` (uncomment the mount in
   `docker-compose.yml` and create that file). Keep `unattended = true`: it tells
   the ring that nobody sits at the hub's screen, so the hub never votes and no
   checkpoint or Red change waits for it (without a mounted file the container
   writes that setting itself):

   ```toml
   [sync]
   unattended = true

   [sync.notify]
   command = "/usr/sbin/sendmail -t -i"
   to = ["you@example.com"]
   from = "sshub-notify@example.com"
   ```

3. Start it. Port 2222 on the host is used by default; set `HUB_PORT` to change it:

   ```sh
   cd deploy/hub
   docker compose up -d --build
   ```

4. Add the hub to your terminal as a normal host: address = this machine, port =
   `HUB_PORT`, user = `hub`, key = the one from step 1.

   A headless hub is enrolled from a terminal, so one human sees both ends:
   `ssh -t hub sshub sync-enroll`.

## Operations

- The host key is kept in the `hub-data` volume at
  `ssh-host-keys/ssh_host_ed25519_key`. Back it up with the volume. Removing the
  volume gives the hub a new identity, and clients will warn about a changed key.
- Logs: `docker compose logs -f hub`.
- Notification command failures are reported by the sender; the message body
  is never written to logs.
