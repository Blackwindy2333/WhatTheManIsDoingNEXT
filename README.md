# WhatTheManDoingNEXT

用 Rust 重构的多平台"在干什么"监控：客户端（Windows / Android）采集设备**当前前台运行的应用**，通过 API 上报远端服务器；服务器接收并整合，网页访问时统一展示。

## 目录结构

```text
Client/    客户端（Rust 重构）
  core/      wtmd-core：配置、协议、隐私、上报、显示名（Windows/Android 共用）
  windows/   wtmd-client-win：前台采集 + 托盘 + 设置窗口
  android/   Kotlin 薄壳 + Rust JNI 核心（无障碍服务读前台应用）
Server/    wtmd-server：接收上报、整合状态、公开 API、SSE、限流、托管 Web/
Web/       网页（源自 WhatTheManDoingWebUI，适当调整）
docs/      API、环境、部署文档
PLAN.md    可行性研究与实施计划（开发参照）
```

## 构建与运行（随实施阶段补充）

- 依赖走国内镜像源，配置见 [.cargo/config.toml](<.cargo/config.toml>) 与 [docs/environment.md](<docs/environment.md>)。
- 服务器：`cargo run -p wtmd-server`（工作区根目录运行，静态托管 `Web/`）。
- Windows 客户端：`cargo run -p wtmd-client-win`。

## 配置文件约定

- 设置**持久化在 JSON 配置文件**中；启动时若无配置文件则**以默认设置创建**，损坏则备份重建。
- 配置文件**有意不进 .gitignore**，随仓库提交（见 PLAN.md §9）。
- ⚠️ 入库的 `api_token` 仅是默认占位，部署后请自行更换。

## 设计参考

UI 风格参照 `D:\TemporaryFiles\UIDevelopInstructions.md`（Apple Design），原生实现允许降级。
