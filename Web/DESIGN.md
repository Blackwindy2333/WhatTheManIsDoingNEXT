# WebUI DESIGN · 在干什么

内部设计说明（实现对齐用）。UI 参考 Apple Design：材质、响应、排版、无障碍；手势弹簧按计划降级。

## Style anchor

- 真实产品：macOS System Settings + Control Center（Sonoma）——半透明工具栏、系统字体、状态胶囊、卡片网格。
- 体感：安静、系统级、信息优先；不是营销落地页。

## Palette

| Token | Light | Dark | 用途 |
|-------|-------|------|------|
| `--bg` | `#F5F5F7` | `#1C1C1E` | 页面底色 |
| `--surface` | `#FFFFFF` | `#2C2C2E` | 卡片/面板 |
| `--ink` | `#1D1D1F` | `#F5F5F7` | 主文字 |
| `--ink-muted` | `#6E6E73` | `#98989D` | 次要文字 |
| `--accent` | `#0071E3` | `#0A84FF` | 主操作/链接/焦点 |
| `--ok` | `#34C759` | `#30D158` | 在线 |
| `--warn` | `#FF9F0A` | `#FF9F0A` | 已暂停 |
| `--danger` | `#FF3B30` | `#FF453A` | 错误/离线/删除 |
| `--hairline` | `rgba(0,0,0,.08)` | `rgba(255,255,255,.12)` | 分割线 |

材质：`backdrop-filter: blur(20px) saturate(180%)` + 半透明底；`prefers-reduced-transparency` 时改实心。

## Typography

- 字体栈：`-apple-system, BlinkMacSystemFont, "SF Pro Text", "PingFang SC", "Segoe UI", sans-serif`
- Display（hero/页标题）：`clamp(1.75rem, 4vw, 2.5rem)` / weight 600 / `letter-spacing: -0.02em` / line-height 1.1
- Title：1.125–1.25rem / 600
- Body：0.9375–1rem / 400 / line-height 1.5 / tracking 0
- Caption：0.75–0.8125rem / 400–500 / 可微正字距
- 层级靠 weight + size + leading，不靠颜色堆砌

## Layout

- 内容最大宽 `1080px`，水平 padding `1.25–2rem`
- 间距节奏：4 / 8 / 12 / 16 / 24 / 32 / 48
- 设备网格：`auto-fill, minmax(260px, 1fr)`，卡片圆角 16px
- 顶栏 sticky、半透明，内容可滚到其下；滚动边缘用渐变遮罩代替硬分割线

## Motion

- 按压：`:active { transform: scale(.97) }`，100ms ease-out（pointer-down 即反馈）
- 卡片入场：opacity 0→1 + scale .98→1，180–220ms，stagger ≤ 30ms
- **降级**：不引入 spring 库；不做 velocity 交接、橡皮筋、可拖拽 sheet
- `prefers-reduced-motion: reduce`：去掉 transform，仅保留 ≤120ms 透明度变化

## Signature moments

1. **状态胶囊**（顶部/卡片角）：圆点 + 文案，在线/离线/已暂停一眼可辨。
2. **玻璃顶栏**：滚动时内容穿行其下，像系统层浮在数据之上。

## Screens

| 屏 | 内容 |
|----|------|
| 公开监控 | 顶栏、设备卡片网格（健康角标、当前应用）、可选历史时间线 |

## Data / interaction

- 浏览器只调本 WebUI 公开 API；设备 `api_token` 不下发。
- 刷新：默认 5s 轮询公开聚合接口；有 SSE 时优先 EventSource，断线回退轮询。
- 配置仅 `config.json`（无管理后台、无在线修改）。

## File map

- `index.html` / `styles.css` / `app.js` — 唯一入口与 UI
- `server/` — FastAPI 聚合代理 + 公开 API
- `config.example.json` — 配置模板（真实 `config.json` 不入库）
