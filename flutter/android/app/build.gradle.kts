plugins {
    id("com.android.application")
    // gradle.properties explicitly opts out of AGP 9 built-in Kotlin for Flutter.
    id("org.jetbrains.kotlin.android")
    id("dev.flutter.flutter-gradle-plugin")
}

val signingValues = listOf(
    "ANDROID_KEYSTORE_PATH", "ANDROID_KEYSTORE_PASSWORD", "ANDROID_KEY_ALIAS", "ANDROID_KEY_PASSWORD"
).associateWith { System.getenv(it).orEmpty() }
val unsignedArtifact = System.getenv("HERMOD_ANDROID_UNSIGNED") == "true"
val hasSigning = signingValues.values.all { it.isNotBlank() }

android {
    namespace = "org.hermod.transfer"
    compileSdk = 36
    ndkVersion = "28.2.13676358"

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    defaultConfig {
        applicationId = "org.hermod.transfer"
        minSdk = 34
        targetSdk = 36
        versionCode = flutter.versionCode
        versionName = flutter.versionName
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }
    signingConfigs {
        if (hasSigning && !unsignedArtifact) {
            create("releaseSecrets") {
                storeFile = file(signingValues.getValue("ANDROID_KEYSTORE_PATH"))
                storePassword = signingValues.getValue("ANDROID_KEYSTORE_PASSWORD")
                keyAlias = signingValues.getValue("ANDROID_KEY_ALIAS")
                keyPassword = signingValues.getValue("ANDROID_KEY_PASSWORD")
            }
        }
    }
    buildTypes {
        release {
            // Deliberately no debug-signing fallback. Unsigned is explicit and not distributable.
            signingConfig = if (hasSigning && !unsignedArtifact) signingConfigs.getByName("releaseSecrets") else null
        }
    }
    packaging { jniLibs { useLegacyPackaging = false } }
}

gradle.taskGraph.whenReady {
    if (allTasks.any { it.name.contains("Release", ignoreCase = true) }) {
        check(unsignedArtifact || hasSigning) { "Release signing Secrets are required; no debug fallback" }
        listOf("arm64-v8a", "x86_64").forEach { abi ->
            check(file("src/main/jniLibs/$abi/libhermod_flutter_bridge.so").isFile) {
                "Missing Rust bridge for $abi; use flutter/scripts/build-android.sh in Actions"
            }
        }
    }
}

kotlin {
    compilerOptions { jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17 }
}

// FlutterFragmentActivity is needed for the AndroidX ActivityResult registry.
dependencies { implementation("androidx.activity:activity-ktx:1.10.1") }

flutter { source = "../.." }
