#!/bin/sh
# Runs INSIDE sshub-hostsync-androidbuild:local with the checkout at /work, cargo target at /target,
# cargo home /cargo-home, gradle home /gradle-home, outputs in /out.
#   build.sh build   core tests, cargo-ndk .so, uniffi Kotlin, debug APK and androidTest APK
#   build.sh test    build + `./gradlew connectedDebugAndroidTest` against ADB_SERVER (host:port)
set -eu
mode=${1:-build}
cd /work
export CARGO_HOME=/cargo-home GRADLE_USER_HOME=/gradle-home CARGO_TARGET_DIR=/target HOME=/home/ci
CORE=crates/sshub-phone-core/Cargo.toml
# Reproducibility: no absolute paths or build ids in the .so; 16 KiB page alignment for Android 15+.
RF="--remap-path-prefix=/work=/src --remap-path-prefix=/cargo-home=/cargo -C link-arg=-Wl,--build-id=none -C link-arg=-Wl,-z,max-page-size=16384"
mkdir -p /out

if [ "$mode" = bootstrap ]; then
    # One-off generation of the committed files: Cargo.lock, Gradle wrapper (checksummed), debug keystore.
    cargo generate-lockfile --manifest-path "$CORE"
    cd android
    gradle --no-daemon --console=plain wrapper --gradle-version 8.11.1 --distribution-type bin \
        --gradle-distribution-sha256-sum f397b287023acdba1e9f6fc5ea72d22dd63669d59ed4a289a29b1a76eee151c6
    [ -f debug.keystore ] || keytool -genkeypair -keystore debug.keystore -storepass android -alias androiddebugkey \
        -keypass android -keyalg RSA -keysize 2048 -validity 36500 -dname "CN=Android Debug,O=Android,C=US"
    exit 0
fi

echo "== versions"; cat /opt/VERSIONS.txt

echo "== core unit tests (host)"
cargo test --locked --manifest-path "$CORE"

echo "== cargo-ndk"
rm -rf android/app/src/main/jniLibs
RUSTFLAGS="$RF" cargo ndk -t arm64-v8a -t x86_64 --platform 30 -o android/app/src/main/jniLibs \
    build --locked --release --manifest-path "$CORE"

echo "== uniffi bindings"
cargo build --locked --manifest-path "$CORE" --features cli --bin uniffi-bindgen
rm -rf android/app/build/generated/uniffi/kotlin
"$CARGO_TARGET_DIR/debug/uniffi-bindgen" generate \
    --library "$CARGO_TARGET_DIR/x86_64-linux-android/release/libsshub_phone_core.so" \
    --language kotlin --out-dir android/app/build/generated/uniffi/kotlin

echo "== gradle"
cd android
./gradlew --no-daemon --console=plain assembleDebug assembleDebugAndroidTest
cp app/build/outputs/apk/debug/app-debug.apk /out/
cp app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk /out/
ls -l /out/*.apk
sha256sum /out/*.apk
unzip -l /out/app-debug.apk | grep -E '\.so$'

if [ "$mode" = test ]; then
    : "${ADB_SERVER:?ADB_SERVER=host:port of the emulator}"
    echo "== adb connect $ADB_SERVER"
    adb start-server
    adb connect "$ADB_SERVER"
    adb -s "$ADB_SERVER" wait-for-device
    export ANDROID_SERIAL="$ADB_SERVER"
    adb -s "$ADB_SERVER" shell getprop ro.build.version.sdk
    echo "== connectedDebugAndroidTest"
    ./gradlew --no-daemon --console=plain connectedDebugAndroidTest
    cp -r app/build/outputs/androidTest-results /out/ 2>/dev/null || true
    cp -r app/build/reports/androidTests /out/ 2>/dev/null || true
    adb disconnect "$ADB_SERVER" || true
fi
