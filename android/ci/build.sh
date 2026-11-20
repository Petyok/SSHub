#!/bin/sh
# Runs INSIDE sshub-hostsync-androidbuild:local with the checkout at /work, cargo target at /target,
# cargo home /cargo-home, gradle home /gradle-home, outputs in /out.
#   build.sh build   core tests, cargo-ndk .so, uniffi Kotlin, debug APK and androidTest APK
#   build.sh native      for focused runs: core tests (full runs only), cargo-ndk x86_64 .so, uniffi Kotlin, lint. No Gradle.
#   build.sh connected   ONE Gradle invocation: assembles the debug and androidTest APKs and runs connectedDebugAndroidTest against the
#                        emulator service ADB_HOST (after `native` or `build` in the same /work). FAILFAST=1 stops at the first failed test.
set -eu
mode=${1:-build}
cd /work
export CARGO_HOME=/cargo-home GRADLE_USER_HOME=/gradle-home CARGO_TARGET_DIR=/target HOME=/home/ci
# Same as org.gradle.jvmargs: Gradle then reuses this JVM instead of forking a second one for the build.
export GRADLE_OPTS="-Xmx3g -Dfile.encoding=UTF-8"
# Per-phase wall times: `ph name` ends the previous phase and starts the next; `ph end` prints the last.
ph() { now=$(date +%s); [ -z "${pn:-}" ] || echo "phase_seconds $pn=$((now - pt))"; pn=$1; pt=$now; }
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
    echo "== connectedDebugAndroidTest (one Gradle invocation: assemble + install + test)"
    ph gradle_connected
    cd android
    rc=0
    set -- ./gradlew --no-daemon --console=plain $GRADLE_EXTRA connectedDebugAndroidTest ${TEST_CLASS:+-Pandroid.testInstrumentationRunnerArguments.class=$TEST_CLASS}
    if [ -n "${FAILFAST:-}" ]; then
        glog=/tmp/gradle-connected.log
        "$@" > "$glog" 2>&1 &
        gp=$!
        tail -n +1 -f "$glog" &
        tp=$!
        while kill -0 "$gp" 2>/dev/null; do
            if hit=$(grep -m1 -E '^[A-Za-z0-9_.$]+ > .*\] FAILED' "$glog"); then
                echo "FAILFAST: first failure: $hit"
                for p in io.github.petyok.sshub io.github.petyok.sshub.test io.github.petyok.sshub.debug io.github.petyok.sshub.debug.test; do adb shell am force-stop "$p" >/dev/null 2>&1 || true; done
                pkill -f GradleWrapperMain || true; pkill -f gradle-launcher || true; kill "$gp" 2>/dev/null || true
                rc=1
                break
            fi
            sleep 2
        done
        wait "$gp" 2>/dev/null || { [ "$rc" -ne 0 ] || rc=$?; }
        kill "$tp" 2>/dev/null || true
    else
        "$@" || rc=$?
    fi
    ph end
    rm -rf /out/shots; cp -r app/build/outputs/connected_android_test_additional_output /out/shots 2>/dev/null || true
    cp -r app/build/outputs/androidTest-results /out/ 2>/dev/null || true
    cp -r app/build/reports/androidTests /out/ 2>/dev/null || true
    [ -f gradle/verification-metadata.xml ] && cp gradle/verification-metadata.xml /out/ || true
    cp app/build/outputs/apk/debug/app-debug.apk /out/ 2>/dev/null || true
    cp app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk /out/ 2>/dev/null || true
    # The check gates the run: a failure turns the result red.
    if [ -f /out/app-debug.apk ]; then PATH=$ANDROID_HOME/build-tools/35.0.0:$PATH sh /work/android/ci/manifest-check.sh /out/app-debug.apk || { [ "$rc" -ne 0 ] || rc=1; }; fi
    exit "$rc"
fi

echo "== versions"; cat /opt/VERSIONS.txt
ph start

# The Rust core: crates/sshub-phone-core in the root workspace (one Cargo.lock with the terminal), built with the `phone` profile
# (release, symbols kept: uniffi library mode reads its metadata from them).
if [ -z "${TEST_CLASS:-}" ]; then
    ph core_unit_tests
    echo "== core unit tests (host)"
    cargo test --locked -p sshub-phone-core --lib
else
    echo "== core unit tests (host) skipped: focused run (TEST_CLASS set)"
fi

ph cargo_ndk
echo "== cargo-ndk"
rm -rf android/app/src/main/jniLibs
# `build` produces the release APK: start from clean inputs (no warm core objects, no warm Gradle outputs), never from incremental state.
if [ "$mode" = build ]; then
    rm -rf /target/aarch64-linux-android/phone /target/x86_64-linux-android/phone
    # android/app/build and android/build are bind mounts (the warm Gradle state of the shared tree, see remote.sh): a mount
    # point cannot be removed ("Device or resource busy"), its contents can.
    for d in android/app/build android/build; do
        [ -d "$d" ] && find "$d" -mindepth 1 -delete
    done
fi
# Focused runs and `native` need only the emulator ABI; `build` (release, device APK) keeps both.
if [ "$mode" = native ]; then NDK_TARGETS="-t x86_64"; else NDK_TARGETS="-t arm64-v8a -t x86_64"; fi
RUSTFLAGS="$RF" cargo ndk $NDK_TARGETS --platform 30 -o android/app/src/main/jniLibs \
    build --locked -p sshub-phone-core --profile phone

ph uniffi_bindgen
echo "== uniffi bindings"
# The bindgen binary only reads the .so, so it is cached by the uniffi version and its source, not rebuilt per run.
ver=$(awk '/^name = "uniffi"$/ { getline; print $3; exit }' Cargo.lock | tr -d '"')
lockhash=$(awk '/^name = "uniffi[a-z_]*"$/ { n=$0; getline v; getline s; print n v s }' Cargo.lock | sha256sum | cut -c1-12)
key="$ver-$(sha256sum crates/sshub-phone-core/src/bin/uniffi-bindgen.rs | cut -c1-12)-$lockhash"
bg=/cargo-home/bin/uniffi-bindgen-$key
if [ ! -x "$bg" ]; then
    echo "building uniffi-bindgen $key"
    cargo build --locked -p sshub-phone-core --features cli --bin uniffi-bindgen
    mkdir -p /cargo-home/bin
    cp "$CARGO_TARGET_DIR/debug/uniffi-bindgen" "$bg.tmp.$$" && mv -f "$bg.tmp.$$" "$bg"
else
    echo "uniffi-bindgen $key cached"
fi
rm -rf android/app/build/generated/uniffi/kotlin
"$bg" generate \
    --library "$CARGO_TARGET_DIR/x86_64-linux-android/phone/libsshub_phone_core.so" \
    --language kotlin --out-dir android/app/build/generated/uniffi/kotlin

ph lint
echo "== lint (colour, ellipsis, logging rules)"
sh /work/android/ci/lint.sh

if [ "$mode" = native ]; then ph end; exit 0; fi

ph gradle_build
echo "== gradle"
cd android
./gradlew --no-daemon --console=plain $GRADLE_EXTRA assembleDebug assembleDebugAndroidTest assembleRelease
cp app/build/outputs/apk/debug/app-debug.apk /out/
cp app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk /out/
cp app/build/outputs/apk/release/app-release-unsigned.apk /out/
[ -f gradle/verification-metadata.xml ] && cp gradle/verification-metadata.xml /out/ || true
ls -l /out/*.apk
sha256sum /out/*.apk
PATH=$ANDROID_HOME/build-tools/35.0.0:$PATH sh /work/android/ci/manifest-check.sh /out/app-debug.apk
PATH=$ANDROID_HOME/build-tools/35.0.0:$PATH APK_KIND=release sh /work/android/ci/manifest-check.sh /out/app-release-unsigned.apk
unzip -l /out/app-debug.apk | grep -E '\.so$' || true
ph end

