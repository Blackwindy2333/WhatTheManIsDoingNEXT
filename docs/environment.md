# 开发环境与镜像源

## 工具链现状（2026-10 检查）

| 工具 | 版本 / 位置 |
|------|-------------|
| Rust | 1.98.1（`x86_64-pc-windows-msvc`，MSVC = VS Community 2026） |
| JDK | Temurin 21，`D:\AndroidDev\jdk-21` |
| Gradle | 8.14.6，`D:\AndroidDev\gradle-8.14.6` |
| Android SDK | `D:\AndroidDev\Android\Sdk`（platforms 35/36/36.1/37.0，build-tools 35–36.1，cmdline-tools，platform-tools） |
| Android NDK | 27.3.13750724、28.2.13676358 |
| 模拟器 | AVD `Pixel_API_36` |
| Python / Node | 3.14 / 24（仅辅助） |

`JAVA_HOME`、`ANDROID_HOME`/`ANDROID_SDK_ROOT` 及 PATH 已写入用户环境变量（新开终端生效）。

## 镜像源（全部依赖获取走国内镜像）

| 生态 | 主镜像 | 备选 | 落点 |
|------|--------|------|------|
| Rust crates | rsproxy.cn 稀疏索引 | USTC `sparse+https://mirrors.ustc.edu.cn/crates.io-index/` | [.cargo/config.toml](<../.cargo/config.toml>)（提交入库） |
| rustup 组件/target | USTC `https://mirrors.ustc.edu.cn/rustup` | Tuna / rsproxy | 安装命令下述 |
| Gradle 发行版 | 腾讯云 mirrors.cloud.tencent.com | 阿里云 | `Client/android/gradle/wrapper/gradle-wrapper.properties` 的 `distributionUrl` |
| Maven/Gradle 依赖 | 阿里云 `maven.aliyun.com`（google / public / gradle-plugin） | 腾讯云 | `Client/android/settings.gradle` + `build.gradle`（提交入库） |
| Android SDK 组件 | 腾讯/阿里云 AndroidSDK 镜像 | 官方 | 仅补装组件时使用 |

### rustup 安卓编译目标安装

```powershell
$env:RUSTUP_DIST_SERVER='https://mirrors.ustc.edu.cn/rustup'
$env:RUSTUP_UPDATE_ROOT='https://mirrors.ustc.edu.cn/rustup/rustup'
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
cargo install cargo-ndk        # 在仓库根目录执行，自动使用 .cargo/config.toml 镜像
```

### 镜像失效处理

crates 镜像不同步时，把 `.cargo/config.toml` 的 `replace-with` 改为 `ustc-sparse`（或反之）；Gradle/Maven 阿里云与腾讯云互换；SDK 组件缺失时用 `sdkmanager` 配镜像代理补装。

## Android 运行环境变量（命令行构建时）

```powershell
$env:JAVA_HOME='D:\AndroidDev\jdk-21'
$env:ANDROID_HOME='D:\AndroidDev\Android\Sdk'
$env:Path="D:\AndroidDev\gradle-8.14.6\bin;$env:Path"
```
