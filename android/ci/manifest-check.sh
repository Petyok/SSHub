#!/bin/sh
# Usage: manifest-check.sh <apk> [aapt2]
# Fails when the APK requests a permission outside the allowed list, or the manifest loses a hardening attribute.
set -eu
apk=${1:?usage: manifest-check.sh <apk> [aapt2]}
aapt2=${2:-${AAPT2:-aapt2}}
if ! command -v "$aapt2" >/dev/null 2>&1 && [ -n "${ANDROID_HOME:-}" ]; then
    aapt2=$(ls -d "$ANDROID_HOME"/build-tools/*/aapt2 2>/dev/null | sort | tail -n 1)
fi

out=$("$aapt2" dump permissions "$apk")
pkg=$(printf '%s\n' "$out" | sed -n "s/^package: //p" | head -n 1)
allowed="android.permission.CAMERA
android.permission.BLUETOOTH_SCAN
android.permission.BLUETOOTH_CONNECT
android.permission.USE_BIOMETRIC
$pkg.DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION"

rc=0
printf '%s\n' "$out" | sed -n "s/^uses-permission[a-z0-9-]*: name='\([^']*\)'.*/\1/p" | while read -r p; do
    printf '%s\n' "$allowed" | grep -qxF "$p" || { echo "FORBIDDEN permission: $p" >&2; echo bad; }
done | grep -q bad && rc=1

if ! printf '%s\n' "$out" | grep "BLUETOOTH_SCAN" | grep -q "neverForLocation"; then
    echo "BLUETOOTH_SCAN must carry usesPermissionFlags neverForLocation" >&2; rc=1
fi

tree=$("$aapt2" dump xmltree --file AndroidManifest.xml "$apk")
for attr in allowBackup usesCleartextTraffic; do
    printf '%s\n' "$tree" | grep -E "$attr" | grep -Eq "=(false|\(type 0x12\)0x0)" || { echo "manifest: $attr must be false" >&2; rc=1; }
done
printf '%s\n' "$tree" | grep -q "dataExtractionRules" || { echo "manifest: dataExtractionRules missing" >&2; rc=1; }

[ "$rc" -eq 0 ] && echo "manifest ok: $pkg"
exit "$rc"
