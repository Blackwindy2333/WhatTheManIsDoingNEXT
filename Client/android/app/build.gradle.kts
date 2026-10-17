// app 模块：Kotlin 薄壳（业务逻辑在 wtmd-core / JNI 桥中）。
plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.whatthemanddoing.app"
    compileSdk = 36
    // 与构建 Rust 桥所用 NDK 一致（NDK 27.3.13750724）
    ndkVersion = "27.3.13750724"

    defaultConfig {
        applicationId = "com.whatthemanddoing.app"
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    buildTypes {
        release {
            // 侧载分发：不做混淆（native 库名/JNI 符号需保持稳定）
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }
}

// ---- Rust JNI 桥构建（cargo ndk -> src/main/jniLibs）----
// 跳过：./gradlew assembleDebug -PskipRust（需已有 jniLibs 产物）
val skipRust: Boolean = project.hasProperty("skipRust")
if (!skipRust) {
    tasks.register<Exec>("buildRust") {
        group = "build"
        description =
            "Build the Rust JNI bridge (cargo ndk) into app/src/main/jniLibs; skip with -PskipRust"
        workingDir = rootProject.file("rust")
        val isWindows = System.getProperty("os.name").orEmpty()
            .startsWith("Windows", ignoreCase = true)
        if (isWindows) {
            commandLine(
                "powershell",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                "build-native.ps1"
            )
        } else {
            commandLine("bash", "build-native.sh")
        }
    }
    tasks.named("preBuild") {
        dependsOn("buildRust")
    }
}

dependencies {
    implementation("com.google.android.material:material:1.12.0")
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("androidx.core:core-ktx:1.13.1")

    // 本地 JVM 单测（纯逻辑：节流、配置字段校验映射、状态格式化）
    testImplementation("junit:junit:4.13.2")
    // 单测环境下提供真实 org.json（android.jar 中同名类是 stub）
    testImplementation("org.json:json:20240303")
}
