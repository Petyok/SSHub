#!/bin/sh
# Boots the headless emulator and exposes its adb port on all interfaces of the compose network.
set -eu

AVD=e2e
CONSOLE_PORT=5560                 # emulator console; adb listens on CONSOLE_PORT+1 (loopback only)
ADB_LOOPBACK_PORT=$((CONSOLE_PORT + 1))
SYSIMG="system-images;android-35;google_apis_playstore;x86_64"

echo "no" | avdmanager create avd --force -n "$AVD" -k "$SYSIMG" -d pixel_6 >/dev/null

# The adb SERVER listens on all interfaces of the (internal) compose network, port 5037. The emulator's own adb
# server is the only authorized client of the guest adbd (the guest is a user build with secure adb), so other
# containers talk to this server (`adb -H android devices`, Playwright `_android.devices({host:'android',port:5037})`),
# not to the guest directly.
adb kill-server
adb -a -P 5037 start-server

emulator -avd "$AVD" -port "$CONSOLE_PORT" \
    -no-window -no-audio -no-boot-anim -no-snapshot -no-metrics \
    -gpu swiftshader_indirect \
    -cores "${EMU_CORES:-4}" -memory "${EMU_MEMORY:-3072}" &
EMU_PID=$!

adb wait-for-device
until [ "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = "1" ]; do
    sleep 2
done
echo "boot_completed=1"

wait "$EMU_PID"
