plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}
android {
    namespace = "org.hermod.documents"
    compileSdk = 37
    buildToolsVersion = "37.0.0"
    defaultConfig { minSdk = 34 }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}
kotlin { compilerOptions { jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17 } }
dependencies {
    implementation("androidx.documentfile:documentfile:1.1.0")
    implementation(project(":tauri-android"))
    implementation("androidx.core:core-ktx:1.19.1")
}
