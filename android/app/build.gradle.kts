import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// Release builds are signed with android/sofa-remote.keystore when it exists (see README),
// so new versions install over old ones; without it they fall back to the debug key.
// The passwords can be overridden with ANDROID_KEYSTORE_PASSWORD / ANDROID_KEY_PASSWORD.
val keystore = rootProject.file("sofa-remote.keystore")
fun env(name: String): String? = System.getenv(name)?.takeIf { it.isNotEmpty() }
val storePass = env("ANDROID_KEYSTORE_PASSWORD") ?: "sofaremote"

android {
    namespace = "app.sofaremote"
    compileSdk = 35

    defaultConfig {
        applicationId = "app.sofaremote"
        minSdk = 26
        targetSdk = 34
        versionCode = 1
        versionName = "1.0"
    }

    signingConfigs {
        create("release") {
            storeFile = keystore
            storePassword = storePass
            keyAlias = env("ANDROID_KEY_ALIAS") ?: "sofa"
            keyPassword = env("ANDROID_KEY_PASSWORD") ?: storePass
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName(if (keystore.exists()) "release" else "debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

kotlin {
    compilerOptions { jvmTarget.set(JvmTarget.JVM_17) }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.0")
    implementation("androidx.media:media:1.7.0")
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    implementation("com.journeyapps:zxing-android-embedded:4.3.0")
}

// Every APK build is copied to release/ at the top of the project, next to the PC binaries:
// assembleRelease -> sofa-remote-android.apk, assembleDebug -> sofa-remote-android-debug.apk
for ((variant, file) in listOf("release" to "sofa-remote-android.apk", "debug" to "sofa-remote-android-debug.apk")) {
    val name = variant.replaceFirstChar(Char::uppercase)
    val copy = tasks.register<Copy>("copy${name}ApkToRelease") {
        from(layout.buildDirectory.file("outputs/apk/$variant/app-$variant.apk"))
        into(rootProject.layout.projectDirectory.dir("../release"))
        rename { file }
    }
    tasks.matching { it.name == "assemble$name" }.configureEach { finalizedBy(copy) }
}
