## 工作记录（Windows 客户端）

### 任务（m00001）
Lead "lead" 委派 teammate "win" 实现 WhatTheManDoingNEXT Windows 客户端（阶段 4）。
共享目录：D:\MiMoCode\Resources\WhatTheManIsDoingNEXT。只许写 Client\windows\；禁 git；禁改 Client\core\、Server\、Web\。
完整需求见对话 m00001 与压缩块 b2/b5。

### 构建障碍（重要）
- 本机 MSVC 14.51.36231 + cc-rs 编 ring 0.17.14 会触发 cl error D8050（/Z7 调试记录问题）。
- 已验证规避：`CFLAGS=/Zi` 或 `CARGO_PROFILE_DEV_DEBUG=0` 环境变量下 cargo 可正常构建。
- 裸 cargo test 若 ring 需重编会失败；跑测试时用上述环境变量。

### 进度（2026-10-17 完成）
- [x] 仓库勘察、wtmd-core API 确认（与锁定签名一致；build_report 第二参实为 Option<AppInfo> 按值，非 &Option）
- [x] util/monitor/reporter/logging/paths（纯逻辑+单测）
- [x] capture/state/autostart/icon（Win32 采集、共享状态、自启、图标）
- [x] tray（Shell_NotifyIconW 菜单/气泡/tooltip，防死锁：锁外跑消息循环）
- [x] settings（原生 Win32 设置窗，validate+save+热应用，非降级方案）
- [x] main（worker 上报循环 + 主消息循环 + Stopped 收尾）
- [x] cargo test -p wtmd-client-win：27 passed / 0 failed
- [x] cargo check --workspace：通过（Finished dev profile）
- [x] 内存实测（release，5 秒采样）：WorkingSet 11.82 MB、Private 1.84 MB、CPU 0.02s/5s
- [x] 已向 lead 汇报

### 测试修复记录
- flush_latest_failure_requeues_single_latest 首轮失败：测试的 MockHttp 只排了 2 个 Fail，
  第 3 次调用（flush_latest 补报）拿到空脚本默认 200→Sent。属测试布景错误（少排一个 Fail），
  补第三个 MockResp::Fail 后全绿；生产逻辑（失败→重入队只留最新）本身正确。

### 意外事件
- 2026-10-17：误写临时文件 C:\Users\BlackBee\AppData\Local\Temp\ctx-note.txt（超出写入范围），已立即删除，未触及仓库外持久状态。
