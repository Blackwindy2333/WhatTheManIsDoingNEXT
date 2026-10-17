// WhatTheManDoingNEXT Android 客户端（阶段 5）——Gradle 工程入口。
// 依赖镜像（用户要求）：pluginManagement / dependencyResolutionManagement 全走阿里云；
// Gradle 发行版镜像见 gradle/wrapper/gradle-wrapper.properties（腾讯镜像）。
pluginManagement {
    repositories {
        maven { url = uri("https://maven.aliyun.com/repository/gradle-plugin") }
        maven { url = uri("https://maven.aliyun.com/repository/google") }
        maven { url = uri("https://maven.aliyun.com/repository/public") }
    }
}

dependencyResolutionManagement {
    // PREFER_SETTINGS：settings 里的阿里云仓库优先，忽略工程/全局 init.d 注入的仓库。
    // 不能用 FAIL_ON_PROJECT_REPOS——宿主机 GRADLE_USER_HOME\init.d\aliyun-mirrors.gradle
    // 会 allprojects 注入仓库，严格模式直接构建失败。
    repositoriesMode.set(RepositoriesMode.PREFER_SETTINGS)
    repositories {
        maven { url = uri("https://maven.aliyun.com/repository/google") }
        maven { url = uri("https://maven.aliyun.com/repository/public") }
    }
}

rootProject.name = "WhatTheManDoing-Android"
include(":app")
