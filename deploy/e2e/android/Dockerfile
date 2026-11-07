# Headless Android 15 (API 35) emulator with the Google Play system image (Chrome preinstalled).
# Needs /dev/kvm at run time; the build itself needs no KVM.
FROM ubuntu@sha256:68ee22004a9109b44973313b02ff7b1de250f1de0f5ecb628d73f579aaff33ac

ARG CMDLINE_TOOLS_BUILD=13114758
ARG CMDLINE_TOOLS_SHA256=7ec965280a073311c339e571cd5de778b9975026cfcbe79f2b1cdcb1e15317ee
ARG SYSTEM_IMAGE=system-images;android-35;google_apis_playstore;x86_64

ENV DEBIAN_FRONTEND=noninteractive \
    ANDROID_HOME=/opt/android-sdk \
    ANDROID_SDK_ROOT=/opt/android-sdk \
    PATH=/opt/android-sdk/cmdline-tools/latest/bin:/opt/android-sdk/platform-tools:/opt/android-sdk/emulator:$PATH

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl unzip openjdk-17-jdk-headless socat \
        libpulse0 libnss3 libglib2.0-0 libdbus-1-3 libfontconfig1 libxkbcommon0 \
        libx11-6 libxcb1 libxext6 libxrender1 libsm6 libice6 libgl1 libxi6 libxrandr2 libxcursor1 libxkbfile1 \
    && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL -o /tmp/cmdline-tools.zip \
        "https://dl.google.com/android/repository/commandlinetools-linux-${CMDLINE_TOOLS_BUILD}_latest.zip" \
    && echo "${CMDLINE_TOOLS_SHA256}  /tmp/cmdline-tools.zip" | sha256sum -c - \
    && mkdir -p "${ANDROID_HOME}/cmdline-tools" \
    && unzip -q /tmp/cmdline-tools.zip -d "${ANDROID_HOME}/cmdline-tools" \
    && mv "${ANDROID_HOME}/cmdline-tools/cmdline-tools" "${ANDROID_HOME}/cmdline-tools/latest" \
    && rm /tmp/cmdline-tools.zip

# Installs whatever the repository serves for these package paths at build time; the resolved
# revisions are recorded in /opt/android-sdk/VERSIONS.txt and printed in the build log.
RUN yes | sdkmanager --licenses >/dev/null \
    && for i in 1 2 3 4 5; do \
         sdkmanager "platform-tools" "emulator" "platforms;android-35" "${SYSTEM_IMAGE}" && break; \
         [ "$i" -lt 5 ] || { echo "sdkmanager failed 5 times" >&2; exit 1; }; \
         echo "sdkmanager attempt $i failed, retrying" >&2; sleep 30; \
       done \
    && { emulator -version | head -n 1; adb version | head -n 1; \
         grep -h '^Pkg' /opt/android-sdk/emulator/source.properties /opt/android-sdk/platform-tools/source.properties; \
         grep -h 'Pkg.Revision' /opt/android-sdk/system-images/android-35/google_apis_playstore/x86_64/source.properties; } \
         > /opt/android-sdk/VERSIONS.txt \
    && cat /opt/android-sdk/VERSIONS.txt

# Fails the build if qemu has unresolved shared libraries. The emulator ships its own libraries (Qt, protobuf, ...) in
# lib64 and its launcher sets LD_LIBRARY_PATH for them, so ldd needs the same path; a bare ldd reports them all missing.
RUN E=/opt/android-sdk/emulator; \
    export LD_LIBRARY_PATH=$E/lib64:$E/lib64/qt/lib:$E/lib64/gles_swiftshader:$E/lib64/vulkan:$E/qemu/linux-x86_64/lib64; \
    ldd $E/qemu/linux-x86_64/qemu-system-x86_64 > /tmp/ldd.txt; \
    if grep 'not found' /tmp/ldd.txt; then echo "unresolved libraries above" >&2; exit 1; fi

COPY start-emulator.sh /usr/local/bin/start-emulator.sh
RUN chmod 0755 /usr/local/bin/start-emulator.sh

HEALTHCHECK --interval=10s --timeout=5s --start-period=300s --retries=3 \
    CMD adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r' | grep -qx 1

ENTRYPOINT ["/usr/local/bin/start-emulator.sh"]
