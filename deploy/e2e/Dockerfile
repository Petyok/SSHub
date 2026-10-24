# Peer image for the host-sync e2e stack (see run.sh). One image serves every
# peer; behaviour differs only by the PEER_NAME environment variable.

FROM rust:1-bookworm AS builder
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev libdbus-1-dev \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
# --no-default-features: links system OpenSSL/libdbus (see Cargo.toml [features]).
RUN cargo build --release --locked --no-default-features --bin sshub \
 && install -m 0755 target/release/sshub /out-sshub

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      openssh-server openssh-client msmtp-mta curl ca-certificates \
      libssl3 libdbus-1-3 sqlite3 jq \
 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /out-sshub /usr/local/bin/sshub

# Unprivileged account; sshd runs as this user on port 2222.
# No SSHUB_*_DIR overrides: those select compat mode, which rejects --profile.
# The peers run the normal profile layout under HOME (default profile).
RUN useradd --create-home --uid 10001 --shell /bin/bash ci \
 && mkdir -p /run/sshd /mesh \
 && chown ci:ci /mesh \
 && install -d -o ci -g ci -m 0700 /home/ci/.ssh /home/ci/ssh \
 && printf 'account default\nhost mailpit\nport 1025\nfrom ci@sshub.e2e\nauth off\ntls off\nlogfile off\n' > /etc/msmtprc \
 && chmod 0644 /etc/msmtprc

COPY deploy/e2e/peer-entrypoint.sh /usr/local/bin/peer-entrypoint.sh
RUN chmod 0755 /usr/local/bin/peer-entrypoint.sh

# No session bus on purpose: keeps the keyring out of reach, so secrets stay in
# the profile directory.
ENV DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent/bus

USER ci
WORKDIR /home/ci
ENTRYPOINT ["/usr/local/bin/peer-entrypoint.sh"]
