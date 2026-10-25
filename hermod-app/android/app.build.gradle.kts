import java.util.Properties
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}
val tauriProperties = Properties().apply { file("tauri.properties").takeIf { it.exists() }?.inputStream()?.use { load(it) } }
val signingFile = System.getenv("ANDROID_KEYSTORE_PATH")
android {
    namespace = "org.hermod.transfer"
    compileSdk = 37
    buildToolsVersion = "37.0.0"
    ndkVersion = "29.0.14206865"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "org.hermod.transfer"
        minSdk = 34
        targetSdk = 37
        versionCode = tauriProperties.getProperty("tauri.android.versionCode").toInt()
        versionName = tauriProperties.getProperty("tauri.android.versionName")
    }
    signingConfigs {
        if (signingFile != null) create("fixedTest") {
            storeFile = file(signingFile)
            storePassword = System.getenv("ANDROID_KEYSTORE_PASSWORD") ?: error("Missing ANDROID_KEYSTORE_PASSWORD")
            keyAlias = System.getenv("ANDROID_KEY_ALIAS") ?: error("Missing ANDROID_KEY_ALIAS")
            keyPassword = System.getenv("ANDROID_KEY_PASSWORD") ?: error("Missing ANDROID_KEY_PASSWORD")
        }
    }
    buildTypes {
        getByName("debug") {
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
        }
        getByName("release") {
            isMinifyEnabled = false
            if (signingFile != null) signingConfig = signingConfigs.getByName("fixedTest")
        }
    }
    compileOptions { sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }
    buildFeatures { buildConfig = true }
    packaging { jniLibs { useLegacyPackaging = false } }
}
kotlin { compilerOptions { jvmTarget = JvmTarget.JVM_17 } }
rust { rootDirRel = "../../../" }
// CLI 的 ARM64 发布构建由固定密钥签名；不允许悄悄输出未签名包。
tasks.configureEach {
    if (name.matches(Regex("pre.*ReleaseBuild"))) doFirst {
        check(signingFile != null) { "Configure fixed Android signing secrets before release" }
        check(name == "preArm64ReleaseBuild" || (name == "preUniversalReleaseBuild" && project.findProperty("targetList") == "aarch64")) { "Only ARM64 release APKs are supported" }
    }
}
dependencies {
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("com.google.android.material:material:1.12.0")
    implementation("androidx.lifecycle:lifecycle-process:2.10.0")
}
apply(from = file("tauri.build.gradle.kts"))
