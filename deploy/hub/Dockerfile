# Hub image: an sshd that terminals reach as a normal sshub host, plus the sshub
# binary and a sendmail shim (msmtp) for notifications. Build from the repository
# root; docker-compose.yml in this directory does that. No secrets are baked in.

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
      openssh-server msmtp-mta ca-certificates libssl3 libdbus-1-3 \
 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /out-sshub /usr/local/bin/sshub
COPY deploy/hub/sshd_config /etc/ssh/sshd_config
COPY deploy/hub/entrypoint.sh /usr/local/bin/hub-entrypoint.sh

# Only the "hub" account can log in, and only with a key from authorized_keys.
RUN useradd --create-home --uid 10001 --shell /bin/bash hub \
 && install -d -o hub -g hub -m 0700 /home/hub/.ssh /home/hub/.config/sshub /home/hub/.local/share/sshub \
 && mkdir -p /run/sshd \
 && chmod 0755 /usr/local/bin/hub-entrypoint.sh

ENV SSHUB_CONFIG_DIR=/home/hub/.config/sshub \
    SSHUB_DATA_DIR=/home/hub/.local/share/sshub

# sshd starts as root (it needs that to log users in) and drops to the account.
EXPOSE 2222
ENTRYPOINT ["/usr/local/bin/hub-entrypoint.sh"]
