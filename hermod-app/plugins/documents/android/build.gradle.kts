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
    // ActivityResult 属于当前模块的编译依赖，不能依赖 app 或 Tauri 的 implementation 传递导出。
    implementation("androidx.activity:activity:1.10.1")
    implementation("androidx.documentfile:documentfile:1.1.0")
    implementation(project(":tauri-android"))
    implementation("androidx.core:core-ktx:1.19.1")
}
