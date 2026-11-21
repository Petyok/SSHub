plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "io.github.petyok.sshub"
    compileSdk = 35
    buildToolsVersion = "35.0.0"
    ndkVersion = "27.2.12479018"

    defaultConfig {
        applicationId = "io.github.petyok.sshub"
        minSdk = 30
        targetSdk = 35
        versionCode = 1
        versionName = "0.0.1-skeleton"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }

    // The public Android debug key, committed on purpose: debug and test builds then carry one
    // stable signing certificate. The release key is the maintainer's and is never in the repo.
    signingConfigs {
        getByName("debug") {
            storeFile = rootProject.file("debug.keystore")
            storePassword = "android"
            keyAlias = "androiddebugkey"
            keyPassword = "android"
        }
    }

    buildTypes {
        // A separate application id so a debug install never replaces or shares data with the release app.
        debug { applicationIdSuffix = ".debug" }
        release { isMinifyEnabled = false; signingConfig = null }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    buildFeatures { compose = true; buildConfig = true }

    // android/ci/build.sh builds libsshub_phone_core.so into jniLibs (cargo-ndk) and writes the
    // uniffi Kotlin bindings here.
    // The terminal-built fixtures of the page (assets/companion/test/fixtures) are the shared vectors of the instrumented core tests.
    sourceSets.getByName("androidTest").assets.srcDir(rootProject.file("../assets/companion/test/fixtures"))
    sourceSets.getByName("main").java.srcDir(layout.buildDirectory.dir("generated/uniffi/kotlin"))
    packaging { resources.excludes += "/META-INF/{AL2.0,LGPL2.1}" }
}

dependencies {
    implementation(platform("androidx.compose:compose-bom:2025.05.01"))
    implementation("androidx.core:core-ktx:1.16.0")
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui")
    // uniffi's generated Kotlin calls the library through JNA.
    implementation("net.java.dev.jna:jna:5.17.0@aar")
    implementation("androidx.navigation:navigation-compose:2.9.0")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.9.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.9.0")
    implementation("androidx.biometric:biometric:1.1.0")
    // biometric 1.1.0 drags in an old Fragment; ActivityResult launchers need 1.3.0 or later (lint: InvalidFragmentVersionForActivityResult)
    implementation("androidx.fragment:fragment:1.8.6")
    implementation("androidx.camera:camera-core:1.4.2")
    implementation("androidx.camera:camera-camera2:1.4.2")
    implementation("androidx.camera:camera-lifecycle:1.4.2")
    implementation("androidx.camera:camera-view:1.4.2")
    implementation("androidx.compose.material3:material3-window-size-class")
    implementation("androidx.compose.ui:ui-tooling-preview")
    debugImplementation("androidx.compose.ui:ui-tooling")

    androidTestImplementation(platform("androidx.compose:compose-bom:2025.05.01"))
    androidTestImplementation("androidx.compose.ui:ui-test-junit4")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test:runner:1.6.2")
    androidTestImplementation("androidx.test:rules:1.6.1")
    androidTestImplementation("androidx.test.uiautomator:uiautomator:2.3.0")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}
