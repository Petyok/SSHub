plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "uk.petruha.sshub.phone"
    compileSdk = 35
    buildToolsVersion = "35.0.0"
    ndkVersion = "27.2.12479018"

    defaultConfig {
        applicationId = "uk.petruha.sshub.phone"
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
        release { isMinifyEnabled = false }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    buildFeatures { compose = true }

    // android/ci/build.sh builds libsshub_phone_core.so into jniLibs (cargo-ndk) and writes the
    // uniffi Kotlin bindings here.
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

    androidTestImplementation(platform("androidx.compose:compose-bom:2025.05.01"))
    androidTestImplementation("androidx.compose.ui:ui-test-junit4")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test:runner:1.6.2")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}
