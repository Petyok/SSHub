// Plugin versions are pinned here; the toolchain (JDK 17, Gradle 8.11.1 via the wrapper, SDK, NDK)
// is pinned by android/docker/Dockerfile. Gradle dependency checksums: gradle/verification-metadata.xml.
plugins {
    id("com.android.application") version "8.10.1" apply false
    id("org.jetbrains.kotlin.android") version "2.1.21" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.1.21" apply false
}
