# WhatTheManDoingNEXT

用 Rust 重构的多平台"在干什么"监控：客户端（Windows / Android）采集设备**当前前台运行的应用**，通过 API 上报远端服务器；服务器接收并整合，网页访问时统一展示。

## 目录结构

```text
Client/    客户端（Rust 重构）
  core/      wtmd-core：配置、协议、隐私、上报、显示名（Windows/Android 共用）
  windows/   wtmd-client-win：前台采集 + 托盘 + 设置窗口（原生 Win32）
  android/   Kotlin 薄壳 + Rust JNI 核心（无障碍服务读前台应用）
Server/    wtmd-server（axum）：接收上报、整合状态、公开 API、SSE、限流、托管 Web/
Web/       网页（源自 WhatTheManDoingWebUI，适当调整）
docs/      API、环境文档（docs/environment.md 含镜像源与工具链）
PLAN.md    可行性研究与实施计划（含实施状态）
```

## 构建与运行

- 依赖走国内镜像源，配置见 [.cargo/config.toml](<.cargo/config.toml>) 与 [docs/environment.md](<docs/environment.md>)。
- **服务器**：`cargo run -p wtmd-server`（工作区根目录运行）。默认监听 `http://127.0.0.1:8080`，静态托管 `Web/`；配置读当前目录 `config.json`，否则 `Server/config.json`（缺失自动按默认创建，损坏改名 `.bak` 重建）。
- **Windows 客户端**：`cargo run -p wtmd-client-win`（或 `cargo build --release -p wtmd-client-win`）。托盘常驻，右键打开设置窗口；配置为可执行文件旁 `config.json`，缺失自动创建。
- **Android 客户端**（详见 [Client/android/README.md](<Client/android/README.md>)）：
  1. `cd Client/android && .\gradlew.bat assembleDebug`（`buildRust` 任务自动 `cargo ndk` 编三 ABI 到 `jniLibs/`，`-PskipRust` 可跳过）；
  2. `adb install -r app\build\outputs\apk\debug\app-debug.apk`；
  3. App 内填服务器地址/设备 ID/token，按引导开启无障碍服务 + 电池白名单（国产 ROM 另需自启动白名单）。

## 测试

| 范围 | 命令 | 结果 |
|---|---|---|
| 共享核心 | `cargo test -p wtmd-core` | 36/36 |
| Windows 客户端 | `cargo test -p wtmd-client-win` | 27/27 |
| 服务器 | `cargo test -p wtmd-server` | 14/14 |
| Android Rust 桥 | `cd Client/android/rust && cargo test` | 19/19 |
| Android Kotlin 单测 | `cd Client/android && .\gradlew.bat testDebugUnitTest` | 22/22 |

端到端验证：客户端上报 → 服务器聚合 → 网页展示，已在 Windows + Android（模拟器）两侧实测通过。

## 配置文件约定

- 设置**持久化在 JSON 配置文件**中；启动时若无配置文件则**以默认设置创建**，损坏则备份重建。
- 配置文件**有意不进 .gitignore**，随仓库提交（见 PLAN.md §9）。
- ⚠️ 入库的 `api_token` 仅是默认占位（`change-me`），部署后请自行更换；服务端 `api_token` 为空字符串时受保护接口一律返回 40100。

## 已知取舍

- **TLS**：Windows 上报 `https://` 可用（native-tls/schannel）。Android 因本机无 perl/make 无法交叉编译 OpenSSL，桥侧用 `native-tls-shim` 编译替身，`https://` 上报暂不可用（`http://` 不受影响），详见 [Client/android/README.md](<Client/android/README.md>)「偏差与取舍」。
- Android UI 相对 Apple Design 指南做了 Material 3 降级（弹簧→ripple、玻璃→elevation 等 9 项），保证原生质感与低内存。

## 设计参考

UI 风格参照 `D:\TemporaryFiles\UIDevelopInstructions.md`（Apple Design），原生实现允许降级。
