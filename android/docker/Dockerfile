# Pinned build image of the native Android companion (docs/host-sync-android.md, "Reproducible build").
# Every download is sha256-checked or fetched by a tool that verifies a signed/checksummed manifest
# (sdkmanager, rustup, crates.io lockfile). Resolved versions are written to /opt/VERSIONS.txt.
# Build:  docker build -t sshub-hostsync-androidbuild:local android/docker   (no KVM needed)
FROM ubuntu@sha256:68ee22004a9109b44973313b02ff7b1de250f1de0f5ecb628d73f579aaff33ac

ARG JDK_URL=https://github.com/adoptium/temurin17-binaries/releases/download/jdk-17.0.21%2B9/OpenJDK17U-jdk_x64_linux_hotspot_17.0.21_9.tar.gz
ARG JDK_SHA256=8b9b611d917e3dfd8419d3566b5949226f69b052fed7e109c1368add1dfe8711
ARG GRADLE_VERSION=8.11.1
ARG GRADLE_SHA256=f397b287023acdba1e9f6fc5ea72d22dd63669d59ed4a289a29b1a76eee151c6
ARG CMDLINE_TOOLS_BUILD=13114758
ARG CMDLINE_TOOLS_SHA256=7ec965280a073311c339e571cd5de778b9975026cfcbe79f2b1cdcb1e15317ee
ARG NDK_VERSION=27.2.12479018
ARG BUILD_TOOLS=35.0.0
ARG PLATFORM=android-35
ARG RUSTUP_VERSION=1.28.2
ARG RUSTUP_INIT_SHA256=20a06e644b0d9bd2fbdbfd52d42540bdde820ea7df86e92e533c073da0cdd43c
ARG RUST_VERSION=1.99.1
ARG CARGO_NDK_VERSION=4.1.2

ENV DEBIAN_FRONTEND=noninteractive \
    JAVA_HOME=/opt/jdk \
    ANDROID_HOME=/opt/android-sdk \
    ANDROID_SDK_ROOT=/opt/android-sdk \
    ANDROID_NDK_HOME=/opt/android-sdk/ndk/${NDK_VERSION} \
    RUSTUP_HOME=/opt/rustup \
    CARGO_HOME=/opt/cargo \
    PATH=/opt/cargo/bin:/opt/jdk/bin:/opt/gradle/bin:/opt/android-sdk/cmdline-tools/latest/bin:/opt/android-sdk/platform-tools:$PATH

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl unzip git build-essential pkg-config \
    && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL -o /tmp/jdk.tgz "${JDK_URL}" \
    && echo "${JDK_SHA256}  /tmp/jdk.tgz" | sha256sum -c - \
    && mkdir /opt/jdk && tar -xzf /tmp/jdk.tgz -C /opt/jdk --strip-components=1 && rm /tmp/jdk.tgz

# Only used to generate the committed wrapper (`gradle wrapper`); builds go through ./gradlew.
RUN curl -fsSL -o /tmp/gradle.zip "https://services.gradle.org/distributions/gradle-${GRADLE_VERSION}-bin.zip" \
    && echo "${GRADLE_SHA256}  /tmp/gradle.zip" | sha256sum -c - \
    && unzip -q /tmp/gradle.zip -d /opt && mv /opt/gradle-${GRADLE_VERSION} /opt/gradle && rm /tmp/gradle.zip

RUN curl -fsSL -o /tmp/cmdline-tools.zip \
        "https://dl.google.com/android/repository/commandlinetools-linux-${CMDLINE_TOOLS_BUILD}_latest.zip" \
    && echo "${CMDLINE_TOOLS_SHA256}  /tmp/cmdline-tools.zip" | sha256sum -c - \
    && mkdir -p "${ANDROID_HOME}/cmdline-tools" \
    && unzip -q /tmp/cmdline-tools.zip -d "${ANDROID_HOME}/cmdline-tools" \
    && mv "${ANDROID_HOME}/cmdline-tools/cmdline-tools" "${ANDROID_HOME}/cmdline-tools/latest" \
    && rm /tmp/cmdline-tools.zip

# sdkmanager verifies each package against the checksum in Google's repository manifest.
RUN yes | sdkmanager --licenses >/dev/null \
    && for i in 1 2 3 4 5; do \
         sdkmanager "platform-tools" "platforms;${PLATFORM}" "build-tools;${BUILD_TOOLS}" "ndk;${NDK_VERSION}" && break; \
         [ "$i" -lt 5 ] || { echo "sdkmanager failed 5 times" >&2; exit 1; }; \
         echo "sdkmanager attempt $i failed, retrying" >&2; sleep 20; \
       done

RUN curl -fsSL -o /tmp/rustup-init \
        "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/x86_64-unknown-linux-gnu/rustup-init" \
    && echo "${RUSTUP_INIT_SHA256}  /tmp/rustup-init" | sha256sum -c - \
    && chmod +x /tmp/rustup-init \
    && /tmp/rustup-init -y --no-modify-path --profile minimal --default-toolchain ${RUST_VERSION} \
         -t aarch64-linux-android -t x86_64-linux-android \
    && rm /tmp/rustup-init \
    && cargo install --locked cargo-ndk --version =${CARGO_NDK_VERSION} \
    && rm -rf /opt/cargo/registry /opt/cargo/git

# socat relays the emulator's adb server (android:5037) to localhost:5037 for Gradle's connected tests.
RUN apt-get update && apt-get install -y --no-install-recommends socat && rm -rf /var/lib/apt/lists/*

RUN { echo "jdk: $(java -version 2>&1 | head -n 1)"; \
      echo "gradle bootstrap: ${GRADLE_VERSION}"; \
      echo "cmdline-tools: ${CMDLINE_TOOLS_BUILD}"; \
      echo "ndk: ${NDK_VERSION}"; echo "build-tools: ${BUILD_TOOLS}"; echo "platform: ${PLATFORM}"; \
      grep -h Pkg.Revision /opt/android-sdk/platform-tools/source.properties; \
      echo "rustc: $(rustc --version)"; echo "cargo-ndk: $(cargo ndk --version)"; \
      rustup target list --installed | sed 's/^/rust target: /'; \
      sha256sum /opt/android-sdk/ndk/${NDK_VERSION}/source.properties; \
    } > /opt/VERSIONS.txt && cat /opt/VERSIONS.txt

# The build runs as an unprivileged user (the toolchains are installed world-readable and are never written to).
RUN useradd --uid 10001 --user-group --create-home --home-dir /home/ci --shell /bin/bash ci
WORKDIR /work
