# WhatTheManDoingNEXT 重构计划：Rust 多平台客户端 + 集中服务器 + Web 展示

> 状态：**已确认（2026-10-17）**，可行性研究与实施方案，供后续开发参照。
> 已确认决策：① 服务端用 Rust 重写；② Windows 客户端 UI = 托盘 + 设置窗口；③ Android 采集以**无障碍服务为主**（UsageStats 转可选降级）；④ **依赖一律使用国内镜像源**。
> 环境结论：安卓环境已配置完毕（JDK 21 / Gradle 8.14.6 / SDK 35–37 / NDK 27.3+28.2 / 模拟器 AVD Pixel_API_36），仅剩 Rust 安卓编译目标与 cargo-ndk 待装。

---

## 1. 原项目现状（调研结论）

| 项目 | 实际角色 | 技术栈 | 关键内容 |
|------|---------|--------|---------|
| `D:\MiMoCode\Resources\WhatTheManDoing` | 设备侧（"客户端"）：前台监控 Agent + 本地控制台 GUI + 本地 JSON API | Python（tkinter、ctypes Win32、FastAPI、SQLite） | `agent/foreground.py`（GetForegroundWindow 轮询）、`agent/reporter.py`（`build_payload`/`send_report`）、`agent/config.py`（配置默认创建）、`server/app/`（envelope、限流、SQLite、`POST /api/v1/report`）、`docs/API.md`（v1 协议） |
| `D:\MiMoCode\Resources\WhatTheManDoingWebUI` | 服务器端只读展示页（"Web端"） | Python FastAPI + 原生 HTML/CSS/JS | `server/`（`/api/public/*`、SSE）、`index.html/app.js/styles.css`（Apple 风 UI）、`DESIGN.md`（设计令牌） |

**协议 v1（保持兼容）**：envelope `{"code":0,"message":"ok","data":...}`；上报体 `{device_id, device_name, status, timestamp, app:{process_name, display_name, window_title}}`；Bearer `api_token`（空 token 一律 401）；429 + `Retry-After` + `X-RateLimit-*`；状态 `active|paused|idle|stopped`；黑名单脱敏 `redacted`；窗口标题默认不上报。

**架构演进**：原——每台设备各自跑 Agent+Server，WebUI 轮询各设备 API；新——**客户端只上报**（Windows/Android → 远端服务器），**服务器集中接收+整合**，**网页访问服务器查看**。

## 2. 目标与成功标准

1. 客户端（Rust 重构）在 **Windows 与 Android** 采集前台应用，经 `POST /api/v1/report` 上报远端服务器。
2. 服务器（Rust 重写）接收上报、整合状态/历史，向网页提供只读 API 与实时推送。
3. 网页（沿用原 WebUI 前端，适当调整）展示各设备"在干什么"。
4. 配置持久化于 JSON 配置文件，启动时缺失则以默认创建；`.gitignore` **不**忽略配置文件。
5. 每个功能完成即 git commit（`feat./fix./bug.` 前缀）；全部代码完成后统一跑测试并修复。
6. UI 参照 `D:\TemporaryFiles\UIDevelopInstructions.md`（Apple Design），原生实现允许降级。
7. 所有依赖走国内镜像源（随仓库提交配置）。

## 3. 可行性结论与技术选型

### 3.1 Android 专项：无障碍服务读前台 + 保活（已调研）

**结论：可行，优于 UsageStats 轮询，采纳为主通道。**

- **读前台**：`AccessibilityService.onAccessibilityEvent` 监听 `TYPE_WINDOW_STATE_CHANGED`，`event.packageName`（+className）即时给出前台应用——事件驱动、低延迟、低功耗；`accessibility-service-config.xml` 配 `accessibilityEventTypes="typeWindowStateChanged"` + `notificationTimeout`（节流 ~300ms）；按包名去重后才上报。
- **隐私**：默认只取包名（`process_name`=包名）；窗口标题仅显式开启时从 source node 根文本尝试读（部分机型拿不到、13+ 脱敏），拿不到即 null，与 Windows 一致。
- **保活（关键收益）**：服务由 SystemServer 经 `AccessibilityServiceConnection` `bindService(BIND_AUTO_CREATE)` 绑定——进程异常退出后**系统自动重启**（`scheduleServiceRestartLocked`，有次数/退避限制）；启用状态持久化在 `Settings.Secure.enabled_accessibility_services`，**重启手机自动恢复**（无需开机广播）；仅 force-stop 移除。是普通应用最强合法保活机制，叠加 ForegroundService（`specialUse`+常驻通知）作用户可见控制面（隐私暂停/状态）。
- **限制**：国产 ROM 默认禁"自启动"会阻碍重启、省电策略等同 force-stop → 首启引导页要求**自启动+电池白名单**；Play 对非无障碍用途极严（stalkerware）→ **仅侧载**（本人自愿分享场景不受影响）。
- 依据：[无障碍服务保活探究](https://github.com/5ec1cff/my-notes/blob/master/keep-accessibility-service-alive.md)、[developer.android.com 无障碍服务](https://developer.android.com/guide/topics/ui/accessibility/views/service?hl=zh-cn)、[ForegroundActivity 双方案对比](https://github.com/ngdathd/ForegroundActivity)。

**对比**：Accessibility（即时/强保活/低功耗/Play 极严） vs UsageStats（1–2s 轮询/无保活/较高功耗/风险略低）→ 前者为主，后者降级可选。

### 3.2 总选型

| 组件 | 选型 | 可行性 |
|------|------|--------|
| Windows 客户端 | **Rust**（`windows` 采集 + `eframe/egui` 设置窗 + `tray-icon` 托盘 + `ureq` 上报） | ✅ 高（Win32 稳定、无需管理员、内存 ~20–40MB、MSVC 已装） |
| 共享核心 | **Rust lib `wtmd-core`**（config/protocol/privacy/report/display-name） | ✅ 高（Android 经 JNI 复用） |
| 服务器 | **Rust**（`axum`+`tokio`+SQLite+SSE+限流+静态托管 Web/） | ✅ 高（单二进制部署） |
| Android 客户端 | **Kotlin 薄壳**（无障碍服务+前台服务+引导+通知+设置页）+ **Rust core cdylib（JNI）** | ⚠️ 中高（厂商碎片与权限引导为主要成本） |
| 网页 | 原生 HTML/CSS/JS 迁移 + 小改 | ✅ 高（`/api/public/*` 路径保持一致） |

## 4. 依赖镜像源配置（随仓库提交）

| 生态 | 镜像 | 落点 |
|------|------|------|
| Rust crates（sparse） | rsproxy.cn 或 USTC（`mirrors.ustc.edu.cn/crates.io-index`） | 仓库根 `.cargo/config.toml`（`[source.crates-io] replace-with`），**提交入库** |
| rustup 组件（含安卓 target） | USTC/Tuna（`RUSTUP_DIST_SERVER`） | `docs/environment.md` 说明 + 阶段 0 安装命令 |
| Gradle 发行版 | 腾讯/阿里云镜像 | `Client/android/gradle/wrapper/gradle-wrapper.properties` 的 `distributionUrl` |
| Maven/Gradle 依赖 | 阿里云 `maven.aliyun.com`（google / public / gradle-plugin 仓） | `Client/android/settings.gradle` + `build.gradle` 的 `repositories`，**提交入库** |
| Android SDK 组件 | 镜像代理（腾讯/阿里云 AndroidSDK） | 仅安装缺组件时使用，记录于 `docs/environment.md` |

原则：镜像配置尽量**提交到仓库**使后续环境一致；单一镜像失效时文档给出切换备选。

## 5. 开发环境现状与缺口（已检查）

**已具备**：Rust 1.98（`x86_64-pc-windows-msvc`+MSVC/VS Community 2026）、Python 3.14、Node 24；JDK 21 Temurin（`D:\AndroidDev\jdk-21`）、Gradle 8.14.6（`D:\AndroidDev\gradle-8.14.6`）、SDK `D:\AndroidDev\Android\Sdk`（platforms 35/36/36.1/37.0、build-tools 35–36.1、cmdline-tools、adb）、NDK 27.3.13750724 与 28.2.13676358、Android Studio + AVD `Pixel_API_36`；`JAVA_HOME`/`ANDROID_HOME`/PATH 已写入用户环境变量（新终端生效）。
**仅剩缺口（阶段 0 补齐，走镜像）**：`rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android`（rustup 镜像）、`cargo install cargo-ndk`（crates 镜像）。

## 6. 目标目录结构（本仓库）

```text
WhatTheManIsDoingNEXT/
├── Cargo.toml                # workspace: Client/core, Client/windows, Server
├── .cargo/config.toml        # crates 镜像源（提交入库）
├── Client/
│   ├── core/                 # wtmd-core（Rust lib + cdylib）：config/protocol/privacy/report/display-name
│   ├── windows/              # wtmd-client-win：采集循环 + 托盘 + 设置窗口 + 开机自启
│   └── android/              # Gradle 工程：Kotlin 壳（无障碍服务/前台服务/引导/通知/设置页）+ JNI
├── Server/                   # wtmd-server：接收上报、整合、公开 API、SSE、限流、托管 Web/
├── Web/                      # 静态前端（源自 WhatTheManDoingWebUI，适当调整）
├── docs/                     # API.md、android-deployment.md、architecture.md、environment.md（镜像/工具链）
├── PLAN.md / README.md
├── .gitignore                # §9：配置文件一律不忽略
└── LICENSE                   # MIT（沿用）
```

> 备注：原始要求只提 Client/ 与 Web/；服务端按"Rust 重写"单列顶层 `Server/`（如需并入 `Web/` 下，实施前一句话即可调整）。

## 7. 分阶段实施计划（每步一 commit，`feat.`/`fix.`/`bug.` 前缀）

### 阶段 0 · 环境与骨架（0.5 人日）
1. 配置全部镜像源（§4）；装 Rust 安卓 targets + cargo-ndk（走镜像）。
2. 建 Cargo workspace 与目录骨架；迁移 LICENSE/.gitattributes；写 `.gitignore`（§9）。
3. `PLAN.md`（本计划）+ `README.md` 骨架 + `docs/environment.md` 提交（`docs.init`）。

### 阶段 1 · 共享核心 `Client/core`（1.5 人日，`feat.core-*`）
- `config.rs`：serde + 全量默认值 + `load_or_create(path)`（缺失→默认创建；损坏→备份 `*.bak` 重建+日志）+ 校验（URL、poll_interval_ms≥100）。字段沿用 `agent/config.json`：`api_base_url, device_id, device_name, api_token, poll_interval_ms, report_window_title(默认false), app_name_blacklist[], privacy_pause, history_limit, display_name_map{}`。
- `protocol.rs`：上报体/envelope/错误码常量（`0/40000/40100/40300/40400/42900/50000`）与 v1 对齐。
- `privacy.rs`：黑名单→`redacted`、`privacy_pause`→`status=paused, app=null`、标题仅显式开启时携带。
- `report.rs`：`ureq` POST `/report` + Bearer；2xx/401/429/5xxx 分类；429 按 `Retry-After` 退避；HTTP 可注入 mock。
- `display_name.rs`：内置映射 + 用户覆盖。
- **测试**：config 默认创建/损坏恢复、payload 隐私矩阵、blacklist、错误分类。

### 阶段 2 · 服务器 `Server/`（3.5 人日，`feat.server-*`）
- `axum + tokio + SQLite`（表 `history(id, device_id, status, process_name, display_name, window_title, timestamp)` + 内存当前状态）。
- 端点（envelope 一致）：`POST /api/v1/report`（Bearer，唯一写入口，广播 SSE）；`GET /api/v1/health`（无鉴权）；`GET /api/v1/status`、`/api/v1/devices`、`/api/v1/devices/{id}`、`/api/v1/devices/{id}/history`（Bearer，兼容 v1）；公开只读 `GET /api/public/config`、`/api/public/probe`、`/api/public/devices`、`/api/public/devices/{id}/history`、`/api/public/stream`（SSE，路径与原 WebUI 一致）；`GET /` 静态托管 `Web/`。
- 全局限流（`X-RateLimit-*`、429+`code=42900`）；`offline_after_seconds` 判离线；`cors_origins`。
- `Server/config.json`：`host, port, db_path, api_token, offline_after_seconds, history_limit, cors_origins, rate_limit_per_minute, page_title, page_subtitle, show_history, refresh_interval_seconds, trust_proxy`——缺失默认创建（网页配置经 `/api/public/config` 下发）。
- **测试**：进程内 app 集成测试（report→devices→history→SSE→限流→401→离线），不手动联调。

### 阶段 3 · Web/ 调整（1 人日，`feat.web-*`）
- 复制 `WhatTheManDoingWebUI/{index.html,styles.css,app.js}` → `Web/`，适当调整：数据源确认同源 `/api/public/*`；删旧部署残留；新增可折叠"如何接入设备"提示块。
- 保持 DESIGN.md 视觉（状态胶囊、玻璃顶栏、卡片网格、SSE 回退轮询、reduced-motion/transparency）；附 `Web/DESIGN.md`。

### 阶段 4 · Windows 客户端 `Client/windows`（3.5 人日，`feat.win-*`）
- **采集循环**（`poll_interval_ms`）：`GetForegroundWindow`→`GetWindowThreadProcessId`→`QueryFullProcessImageNameW`→exe 名为 `process_name`；`GetWindowTextW` 仅显式开启；`GetLastInputInfo` 超阈值→`idle`，锁屏并入 idle；窗口变化才 POST + 心跳补报。
- **上报**：走 `wtmd-core`；断网环形队列缓存（默认 100 条），恢复后报最新。
- **托盘**（`tray-icon`）：状态色图标（绿在线/黄暂停/灰停止），菜单：设置 / 隐私暂停 / 开机自启 / 退出。
- **设置窗口**（`eframe/egui`）：服务器地址、device_id/name、api_token、采样间隔、标题开关、黑名单、隐私暂停、开机自启、测试连接、最近错误。Apple 指南**降级**实现：系统字体、卡片分区、状态胶囊、按压微缩、深浅色跟随系统。
- **开机自启**：`HKCU\...\Run`。
- **测试**：采集/上报/配置单测（Win32 注入缝 mock；GUI 主循环不进单测）。

### 阶段 5 · Android 客户端 `Client/android`（5.5 人日，`feat.android-*`）
- Gradle + Kotlin（minSdk 26 / targetSdk 34）；`wtmd-core` 编 `libwtmd_core.so`（cargo-ndk + JNI：配置读写、`build_payload`、`send_report`）；依赖走阿里云镜像（§4）。
- **无障碍服务**（主通道，§3.1）：`TYPE_WINDOW_STATE_CHANGED`→前台包名→节流去重→JNI 上报；承担保活骨架（系统自动重启、开机自动恢复）。
- **前台服务**（`specialUse`+常驻通知）：上报循环宿主、隐私暂停快捷键、状态展示；`POST_NOTIFICATIONS`(13+)、`INTERNET`。
- **首次引导页**：无障碍开启引导（跳设置+判定）、厂商自启动/电池白名单说明、未启用时明示不产生假数据。
- **设置页**（Material 3，气质参照 Apple 指南）：服务器地址 / device_id / token / 采样间隔 / 黑名单 / 隐私暂停 / 权限状态。
- 产物：侧载 APK + `docs/android-deployment.md`（开启步骤、厂商适配表、不上 Play 的政策声明）。
- **测试**：JUnit 纯逻辑 + Rust core 单测；AVD `Pixel_API_36`/真机手动验收记录。

### 阶段 6 · 测试收尾与文档（2.5 人日，`fix.*`/`docs.*`）
- **全部代码完成后**统一 `cargo test` + `gradlew test`，修复失败。
- 端到端验收：Windows→服务器→网页；Android 侧载→无障碍授权→上报→网页显示。
- README（部署、反代 HTTPS、更换 `api_token` 提醒）、docs/API.md 更新。

### 里程碑依赖
`阶段0 → 阶段1 → {阶段2 → 阶段3, 阶段4, 阶段5} → 阶段6`；3/4/5 可并行；4/5 依赖 core 与协议定稿。

## 8. 数据契约摘要（v1 保持兼容）

```json
// POST /api/v1/report  (Authorization: Bearer <api_token>)
{"device_id":"my-pc","device_name":"My PC","status":"active",
 "timestamp":"2026-01-01T12:00:00Z",
 "app":{"process_name":"Code.exe","display_name":"Visual Studio Code","window_title":null}}
// 响应统一 envelope
{"code":0,"message":"ok","data":{"ok":true}}
```
- Android `process_name`=包名、`window_title` 默认 `null`；`status`: `active|paused|idle`（`paused`/`idle` 时 `app=null`）；黑名单→`redacted`。
- 公开 API 结构与原 WebUI `server/` 一致（`devices[]` 含 `online` 与健康角标字段）。

## 9. 配置与 .gitignore 约定（硬性要求）

- **配置文件一律不进 .gitignore**（`Server/config.json`、客户端 `config.json`、`config.example.json` 全部入库）；忽略项仅：构建产物（`target/`、`build/`、`__pycache__/`）、运行数据（`*.db`、`logs/`、`*.log`）、密钥材料（`*.pem/*.key`、`.env*`）、IDE/OS 杂项。
- 设置持久化于 JSON；**启动时检测配置文件，缺失以默认创建**，损坏则备份重建（两端一致）。
- 提醒：真实 token 入库请自行更换（README 警示沿用原项目）。

## 10. 边界情况与失败模式

| 场景 | 行为 |
|------|------|
| 配置缺失/损坏 | 默认创建 / 备份重建 + 日志 |
| token 空或错误 | 受保护接口一律 401 `code=40100` |
| 429 限流 | 客户端按 `Retry-After` + 指数退避 |
| 断网/服务器宕机 | 重试 + 环形队列缓存；网页显示离线 |
| 端口占用 | 服务器启动明确报错、非零退出 |
| Windows 锁屏/无输入 | `idle`，`app=null` |
| Android 未开无障碍 | 引导页 + 通知明示，不产生假数据 |
| Android ROM 杀后台/禁自启 | 心跳超时→网页离线；引导白名单；系统重启有次数/退避限制 |
| Android 事件风暴/多窗口 | `notificationTimeout` 节流 + 包名去重 |
| 镜像源失效/不同步 | 文档记录备选镜像与切换方法（§4） |
| 时钟 | timestamp 统一 UTC ISO-8601（`Z`） |
| 标题隐私 | 默认不采集不上报；网页以服务端返回字段为准 |

## 11. 复杂度评估（人日）

| 工作项 | 估计 |
|--------|------|
| 环境+镜像+骨架+gitignore | 0.5 |
| 共享核心 core | 1.5 |
| Rust 服务器 | 3.5 |
| Web 调整 | 1 |
| Windows 客户端 | 3.5 |
| Android 客户端 | 5.5 |
| 测试收尾+文档+验收 | 2.5 |
| **合计** | **约 18 人日**（风险缓冲后 20–24） |

## 12. 额外功能建议

**首版内含**：离线环形队列与自动重连；idle 检测；SSE 实时+轮询回退；历史时间线；Windows 开机自启；Android 隐私暂停通知快捷键、开机自动恢复；深浅色跟随系统。
**可选增强（另行确认）**：① Android UsageStats 降级通道；② 每设备独立 token；③ 网页访问密码；④ 使用时长统计图表；⑤ 历史 CSV 导出；⑥ 设备上下线通知；⑦ Windows `SetWinEventHook` 近实时；⑧ Android 快捷设置磁贴；⑨ 报告批量补传；⑩ 多语言。

## 13. 明确假设

1. "客户端"重构 = 原 WhatTheManDoing 设备侧能力；其本地 API 能力由新中央服务器承接；原 `web/` 示例页废弃。
2. 新代码全部落在本工作区仓库；两个原项目目录只读参照，不修改。
3. 协议保持 v1 兼容（envelope、payload、Bearer、`/api/public/*`）。
4. 仅 Windows + Android 为支持目标；服务器部署任意主机，HTTPS 交反代；Android 仅侧载。
5. 测试纪律：开发中期不手动联调，以自动化测试为准；全部完成后统一跑测试修复。
6. 每个功能一个 commit（`feat./fix./bug.` 等前缀）。
7. Android 采集以无障碍服务为主通道，UsageStats 为可选降级。
8. 依赖获取全部走国内镜像源，配置随仓库提交。
