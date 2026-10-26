#!/bin/sh
# Runs INSIDE sshub-hostsync-androidbuild:local with the checkout at /work, cargo target at /target,
# cargo home /cargo-home, gradle home /gradle-home, outputs in /out.
#   build.sh build   core tests, cargo-ndk .so, uniffi Kotlin, debug APK and androidTest APK
#   build.sh connected   `./gradlew connectedDebugAndroidTest` against the emulator service ADB_HOST (after `build` in the same /work)
set -eu
mode=${1:-build}
cd /work
export CARGO_HOME=/cargo-home GRADLE_USER_HOME=/gradle-home CARGO_TARGET_DIR=/target HOME=/home/ci
# Reproducibility: no absolute paths or build ids in the .so; 16 KiB page alignment for Android 15+.
RF="--remap-path-prefix=/work=/src --remap-path-prefix=/cargo-home=/cargo -C link-arg=-Wl,--build-id=none -C link-arg=-Wl,-z,max-page-size=16384"
mkdir -p /out

if [ "$mode" = bootstrap ]; then
    # One-off generation of the committed files: Gradle wrapper (checksummed), debug keystore.
    cd android
    gradle --no-daemon --console=plain wrapper --gradle-version 8.11.1 --distribution-type bin \
        --gradle-distribution-sha256-sum f397b287023acdba1e9f6fc5ea72d22dd63669d59ed4a289a29b1a76eee151c6
    [ -f debug.keystore ] || keytool -genkeypair -keystore debug.keystore -storepass android -alias androiddebugkey \
        -keypass android -keyalg RSA -keysize 2048 -validity 36500 -dname "CN=Android Debug,O=Android,C=US"
    exit 0
fi

if [ "$mode" = connected ]; then
    # Instrumented tests against the emulator project: its adb SERVER (authorized for the guest) is relayed to localhost.
    : "${ADB_HOST:?ADB_HOST=emulator service host (android)}"
    echo "== adb via $ADB_HOST:5037"
    socat TCP-LISTEN:5037,fork,reuseaddr "TCP:$ADB_HOST:5037" &
    sleep 1
    n=0; until adb devices | grep -q 'device$'; do n=$((n + 1)); [ "$n" -le 30 ] || { adb devices; echo "no device" >&2; exit 1; }; sleep 2; done
    adb devices
    for p in ro.build.version.release ro.build.version.sdk ro.product.cpu.abi ro.build.tags; do echo "$p=$(adb shell getprop $p | tr -d '\r')"; done
    echo "chrome: $(adb shell dumpsys package com.android.chrome | grep -m1 versionName | tr -d '\r')"
    echo "== connectedDebugAndroidTest"
    cd android
    rc=0
    ./gradlew --no-daemon --console=plain connectedDebugAndroidTest ${TEST_CLASS:+-Pandroid.testInstrumentationRunnerArguments.class=$TEST_CLASS} || rc=$?
    mkdir -p /out/shots && adb exec-out run-as uk.petruha.sshub.phone tar -cf - -C files shots 2>/dev/null | tar -xf - -C /out 2>/dev/null || true
    cp -r app/build/outputs/androidTest-results /out/ 2>/dev/null || true
    cp -r app/build/reports/androidTests /out/ 2>/dev/null || true
    exit "$rc"
fi

echo "== versions"; cat /opt/VERSIONS.txt

# The Rust core: crates/sshub-phone-core in the root workspace (one Cargo.lock with the terminal), built with the `phone` profile
# (release, symbols kept: uniffi library mode reads its metadata from them).
echo "== core unit tests (host)"
cargo test --locked -p sshub-phone-core --lib

echo "== cargo-ndk"
rm -rf android/app/src/main/jniLibs
RUSTFLAGS="$RF" cargo ndk -t arm64-v8a -t x86_64 --platform 30 -o android/app/src/main/jniLibs \
    build --locked -p sshub-phone-core --profile phone

echo "== uniffi bindings"
cargo build --locked -p sshub-phone-core --features cli --bin uniffi-bindgen
rm -rf android/app/build/generated/uniffi/kotlin
"$CARGO_TARGET_DIR/debug/uniffi-bindgen" generate \
    --library "$CARGO_TARGET_DIR/x86_64-linux-android/phone/libsshub_phone_core.so" \
    --language kotlin --out-dir android/app/build/generated/uniffi/kotlin

echo "== lint (colour, ellipsis, logging rules)"
sh /work/android/ci/lint.sh

echo "== gradle"
cd android
./gradlew --no-daemon --console=plain assembleDebug assembleDebugAndroidTest
cp app/build/outputs/apk/debug/app-debug.apk /out/
cp app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk /out/
ls -l /out/*.apk
sha256sum /out/*.apk
PATH=$ANDROID_HOME/build-tools/35.0.0:$PATH sh /work/android/ci/manifest-check.sh /out/app-debug.apk
unzip -l /out/app-debug.apk | grep -E '\.so$' || true

