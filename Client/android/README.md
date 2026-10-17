# WhatTheManDoingNEXT — Android 客户端（阶段 5）

Rust 核心（wtmd-core）+ Kotlin 薄壳（JNI）。前台应用监听走无障碍服务
（TYPE_WINDOW_STATE_CHANGED，仅包名与窗口类名，不读窗口内容），经
`POST {api_base_url}/api/v1/report`（Bearer token，JSON）上报自建服务器。

> 仅侧载分发。Play 政策不允许非无障碍用途的前台应用监听（Stalkerware 风险），
> 不要上架 Google Play。安装前请告知被监控设备的使用者并获得同意。

## 目录结构

```
Client/android/
├── settings.gradle.kts        # 插件/依赖仓库 = 阿里云镜像
├── build.gradle.kts           # AGP 8.9.2 + Kotlin 2.0.21
├── gradle.properties
├── gradle/wrapper/            # distributionUrl = 腾讯镜像
├── app/                       # Kotlin 薄壳
│   └── src/main/java/com/whatthemanddoing/app/
│       ├── NativeBridge.kt           # JNI 门面
│       ├── MainActivity.kt           # 设置页（Material 3）
│       ├── ForegroundWatchService.kt # 无障碍服务：前台应用监听
│       ├── ForegroundService.kt      # 常驻通知 + 暂停/停止控制面
│       ├── BootReceiver.kt           # 开机引导
│       ├── EventThrottle.kt          # 事件节流（纯逻辑）
│       ├── ConfigMapper.kt           # 表单↔配置映射（纯逻辑）
│       └── StatusFormatter.kt        # 状态 JSON→文本（纯逻辑）
└── rust/                      # JNI 桥 crate（wtmd-android-bridge）
    ├── Cargo.toml             # [workspace] 空表脱离根 workspace；[patch.crates-io] native-tls 替身
    ├── src/{lib.rs,state.rs,service.rs}
    ├── build-native.ps1       # cargo ndk -> app/src/main/jniLibs（Windows）
    ├── build-native.sh        # 同上（macOS/Linux）
    └── vendor/native-tls-shim/  # 编译期替身（见下文「TLS 说明」）
```

## 构建

前置：JDK 21、Android SDK（platform 36）、NDK 27.3.13750724、
rustup 目标 `aarch64-linux-android` / `armv7-linux-androideabi` / `x86_64-linux-android`、
cargo-ndk 4.x。

```powershell
# 1) Rust 桥 -> app/src/main/jniLibs（三 ABI，release）
powershell -ExecutionPolicy Bypass -File Client\android\rust\build-native.ps1
# （macOS/Linux：bash Client/android/rust/build-native.sh）
# 也可跳过：./gradlew assembleDebug -PskipRust（需已有 jniLibs 产物）

# 2) APK（Gradle 会自动先跑 buildRust）
cd Client\android
.\gradlew.bat assembleDebug        # 产物 app\build\outputs\apk\debug\app-debug.apk

# 3) 测试
cd Client\android\rust
cargo test                         # Rust 桥纯逻辑单测
cd ..\
.\gradlew.bat testDebugUnitTest    # Kotlin 纯逻辑单测（JVM）
```

crates 依赖走仓库根 `.cargo/config.toml`（rsproxy 镜像）；Gradle 插件与
依赖走阿里云镜像（settings.gradle.kts）；Gradle 发行版走腾讯镜像
（gradle/wrapper/gradle-wrapper.properties）。

## TLS 说明（与 wtmd-core 现状一致）

wtmd-core 的 `UreqHttp` 使用 `ureq = { default-features = false, features = ["native-tls"] }`，
而 ureq 2.x 在**只开 `native-tls` feature** 时 `default_tls_config()` 实际返回
`NoTlsConfig`——即 `https://` 上报在当前 core 实现下本来就不可用（Windows 亦然），
`http://` 上报不受影响。Android 交叉编译真正的 OpenSSL 需要 perl/make，本机不具备，
故本工程用 `[patch.crates-io]` 将 `native-tls` 替换为 `vendor/native-tls-shim`
（编译期替身，运行期行为与上述现状 1:1 等价，零 C 依赖）。

后续若 core 启用 ureq 的 `tls`（rustls）feature 或做 target-specific 分支，
删除 `[patch.crates-io]` 段即可恢复真实 TLS。

## 安装到设备

```powershell
adb install -r app\build\outputs\apk\debug\app-debug.apk
```

1. 打开 App → 「① 启用无障碍服务」→ 在「已下载的服务」中开启
   **WhatTheManDoing 前台应用监听**（系统自动拉活并跨重启保持）。
2. 「② 电池优化白名单」→ 允许（防国产 ROM 清理后台）。
3. 「③ 厂商自启动设置」→ 开启自启动/后台无限制（小米/华为/OPPO/vivo/三星等）。
4. 填写服务器地址、设备 ID、API Token → 保存。
5. 常驻通知显示当前上报状态；动作按钮可暂停/恢复上报、停止采集。

## 生命周期与保活

- 无障碍服务由 SystemServer `bindService` 绑定：进程被杀自动重启，启用态跨重启
  持久化（无需代码保活）。
- `ForegroundService`（`specialUse` + 常驻通知）承载状态展示与控制；开机
  （BOOT_COMPLETED）时若无障碍已启用会重新拉起，否则发引导通知。
- 进程退出/任务移除前尽力上报 `stopped`（1.5s 宽限）。
- 断网：Rust 上报线程退避重试（1s 起指数，封顶 60s，429 按 Retry-After），
  环形队列（100 条）缓存，恢复后补报最新一条。

## UI 降级清单（Apple Design → Material 3）

| Apple 指南 | 本实现 | 说明 |
|---|---|---|
| 弹簧动效（damping/response） | 系统 ripple + FastOutSlowIn | Material 组件默认动效 |
| backdrop-filter 玻璃材质 | 纯色 surface + elevation 卡片 | M3 Card 默认 |
| 按压 scale(0.97) | ripple 按压反馈 | 按压即时反馈保留，形变省略 |
| 1:1 拖拽/动量投影/rubber-banding | 不适用 | 设置页为纯表单，无自定义手势 |
| 字体/光学字号（SF tracking） | 系统字体 + sp | Dynamic Type 自动跟随 |
| reduced-motion | 跟随系统动画缩放 | 系统「移除动画」自动生效 |
| 深浅色 | DayNight 主题 | 跟随系统 |
| haptic 反馈 | 系统默认点击触感 | 未额外定制 |
| 状态胶囊 | M3 文本/卡片近似 | 未定制 Chip 样式 |

## 隐私

- 仅采集包名与窗口类名（`canRetrieveWindowContent=false`）。
- 窗口标题（Android 为 Activity 类名）默认不上报（`report_window_title=false`）。
- 黑名单/隐私暂停/显示名映射全部在设备端 wtmd-core 完成；隐私暂停时上报
  `status=paused` 且 `app=null`。
