#!/bin/sh
# Boots the headless emulator and exposes its adb port on all interfaces of the compose network.
set -eu

AVD=e2e
CONSOLE_PORT=5560                 # emulator console; adb listens on CONSOLE_PORT+1 (loopback only)
ADB_LOOPBACK_PORT=$((CONSOLE_PORT + 1))
SYSIMG="system-images;android-35;google_apis_playstore;x86_64"

echo "no" | avdmanager create avd --force -n "$AVD" -k "$SYSIMG" -d pixel_6 >/dev/null

adb start-server

emulator -avd "$AVD" -port "$CONSOLE_PORT" \
    -no-window -no-audio -no-boot-anim -no-snapshot -no-metrics \
    -gpu swiftshader_indirect \
    -cores "${EMU_CORES:-4}" -memory "${EMU_MEMORY:-3072}" &
EMU_PID=$!

# ponytail: plain socat relay, no auth. Only reachable on the internal compose network.
socat TCP-LISTEN:5555,fork,reuseaddr TCP:127.0.0.1:"$ADB_LOOPBACK_PORT" &

adb wait-for-device
until [ "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = "1" ]; do
    sleep 2
done
echo "boot_completed=1"

wait "$EMU_PID"
