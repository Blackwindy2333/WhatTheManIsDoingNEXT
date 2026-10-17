//! wtmd-client-win：Windows 客户端入口。
//! 主线程 = 托盘 + 设置窗消息循环；工作线程 = 前台采集 + 上报循环。
//! 采集轮询用 Condvar::wait_timeout（睡眠等待，CPU≈0）。

mod autostart;
mod capture;
mod icon;
mod logging;
mod monitor;
mod paths;
mod reporter;
mod settings;
mod state;
mod tray;
mod util;

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MessageBoxW, TranslateMessage, MB_ICONERROR, MB_OK, MSG,
};

use wtmd_core::config::AppConfig;
use wtmd_core::display_name::resolve_display_name;
use wtmd_core::privacy::build_report;
use wtmd_core::protocol::{now_utc_iso, AppInfo, DeviceStatus, ReportPayload};
use wtmd_core::report::{send_report, UreqHttp};

use crate::monitor::{ChangeDetector, FgState};
use crate::reporter::{Reporter, SubmitOutcome};
use crate::state::AppShared;

/// 直接构造状态载荷（Idle/Stopped：app=None，不走 build_report 的
/// privacy_pause 覆盖，保证状态如实上报；Active 路径仍由 build_report
/// 应用隐私规则——隐私暂停、黑名单、标题开关）。
fn direct_payload(cfg: &AppConfig, status: DeviceStatus) -> ReportPayload {
    ReportPayload {
        device_id: cfg.device_id.clone(),
        device_name: cfg.device_name.clone(),
        status,
        timestamp: now_utc_iso(),
        app: None,
    }
}

/// 按上报结果更新共享的上报状态文案与（一次性）token 警告气泡。
fn apply_outcome(
    shared: &AppShared,
    outcome: &SubmitOutcome,
    queued: usize,
    token_warned: &mut bool,
) {
    let note = match outcome {
        SubmitOutcome::Sent => {
            *token_warned = false;
            "已上报".to_string()
        }
        SubmitOutcome::Unauthorized => {
            if !*token_warned {
                shared.balloon(
                    "token 无效",
                    "api_token 校验失败（401/403），请在「设置」中更新",
                );
                *token_warned = true;
            }
            "token 无效（401/403），60 秒后重试".to_string()
        }
        SubmitOutcome::BadRequest => "上报被服务端拒绝（400），已丢弃".to_string(),
        SubmitOutcome::RateLimited => "被限流（429），稍后重试".to_string(),
        SubmitOutcome::FailedQueued => format!("网络失败，已缓存 {queued} 条"),
    };
    {
        let mut st = shared.lock();
        st.report_note = note;
    }
    tray::request_refresh();
}

/// 采集 + 上报循环（工作线程）。
fn worker(shared: Arc<AppShared>) {
    let http = UreqHttp;
    let mut detector = ChangeDetector::new();
    let mut reporter = Reporter::new();
    let mut last_event = Instant::now();
    let mut stopped_sent = true; // 启动时未停止：无需补发 Stopped
    let mut token_warned = false;
    let mut prev_pause: Option<bool> = None;

    logging::info("上报循环已启动");
    loop {
        // ── 等待一个轮询周期（或被命令唤醒）──
        {
            let st = shared.lock();
            if st.shutdown {
                break;
            }
        }
        let poll = {
            let st = shared.lock();
            Duration::from_millis(st.cfg.poll_interval_ms.max(100))
        };
        {
            let st = shared.lock();
            let st = shared.wait_timeout(st, poll);
            if st.shutdown {
                drop(st);
                break;
            }
        }

        let (cfg, running) = {
            let st = shared.lock();
            (st.cfg.clone(), st.capture_running)
        };

        // ── 托盘"停止采集"：仅报一次 Stopped，随后停摆 ──
        if !running {
            if !stopped_sent {
                let payload = direct_payload(&cfg, DeviceStatus::Stopped);
                let now = Instant::now();
                let outcome = if reporter.can_send(now) {
                    reporter.submit(&http, &cfg, payload, now)
                } else {
                    reporter.enqueue(payload);
                    SubmitOutcome::FailedQueued
                };
                apply_outcome(&shared, &outcome, reporter.queued_len(), &mut token_warned);
                stopped_sent = true;
                {
                    let mut st = shared.lock();
                    st.status = DeviceStatus::Stopped;
                    st.current_app = None;
                }
                logging::info("已停止采集，上报一次 Stopped 状态");
                tray::request_refresh();
            }
            continue;
        }

        if stopped_sent {
            // 恢复采集：重置变化检测，首采样必报。
            stopped_sent = false;
            detector = ChangeDetector::new();
            last_event = Instant::now();
            logging::info("已恢复采集");
        }

        // ── 前台采样 + 变化/心跳判定 ──
        let sample = capture::sample_foreground();
        let state = monitor::classify(sample.as_ref());
        let changed = detector.changed(state, sample.as_ref());
        let now = Instant::now();
        let heartbeat = monitor::heartbeat_due(last_event, now);
        let pause_changed = match prev_pause {
            Some(p) => p != cfg.privacy_pause,
            None => false,
        };
        prev_pause = Some(cfg.privacy_pause);

        // ── 构造载荷（Active 走 build_report 应用隐私规则）──
        let payload = match state {
            FgState::Active => {
                let s = match sample.as_ref() {
                    Some(s) => s,
                    None => continue,
                };
                let app = AppInfo {
                    process_name: s.process_name.clone(),
                    display_name: Some(resolve_display_name(
                        &s.process_name,
                        &cfg.display_name_map,
                    )),
                    window_title: s.window_title.clone(),
                };
                build_report(&cfg, Some(app), DeviceStatus::Active)
            }
            FgState::Idle => direct_payload(&cfg, DeviceStatus::Idle),
        };

        // ── 更新共享展示状态（托盘 tooltip / 状态菜单）──
        {
            let mut st = shared.lock();
            st.status = payload.status;
            st.current_app = payload.app.as_ref().map(|a| {
                a.display_name
                    .clone()
                    .unwrap_or_else(|| a.process_name.clone())
            });
        }
        tray::request_refresh();

        if !(changed || heartbeat || pause_changed) {
            continue;
        }
        last_event = now;

        // ── 上报：退避期只入队；有缓存先 pop_latest 补报最新一条 ──
        if !reporter.can_send(now) {
            reporter.enqueue(payload);
            apply_outcome(
                &shared,
                &SubmitOutcome::FailedQueued,
                reporter.queued_len(),
                &mut token_warned,
            );
            continue;
        }
        let outcome = if reporter.has_queued() {
            reporter.enqueue(payload);
            reporter
                .flush_latest(&http, &cfg, now)
                .unwrap_or(SubmitOutcome::FailedQueued)
        } else {
            reporter.submit(&http, &cfg, payload, now)
        };
        match outcome {
            SubmitOutcome::Sent => {
                if reporter.has_queued() {
                    logging::info(&format!("补报完成，仍有缓存 {} 条", reporter.queued_len()));
                }
            }
            other => {
                logging::error(&format!("上报未成功：{other:?}"));
            }
        }
        apply_outcome(&shared, &outcome, reporter.queued_len(), &mut token_warned);
    }

    // ── 进程退出：尽力而为补报一次 Stopped（失败忽略）──
    if !stopped_sent {
        let cfg = shared.lock().cfg.clone();
        let payload = direct_payload(&cfg, DeviceStatus::Stopped);
        match send_report(&UreqHttp, &cfg, &payload) {
            Ok(_) => logging::info("退出前已上报 Stopped 状态"),
            Err(e) => logging::error(&format!("退出前上报 Stopped 失败（忽略）：{e:?}")),
        }
    }
    logging::info("上报循环已结束");
}

fn main() {
    logging::init(&paths::log_path());
    logging::info(&format!(
        "wtmd-client-win 启动（配置：{}）",
        paths::config_path().display()
    ));

    // 配置：缺失自动创建默认；损坏由 core 备份重建；语义非法则提示后用默认。
    let config_path = paths::config_path();
    let cfg = match AppConfig::load_or_create(&config_path) {
        Ok(cfg) => cfg,
        Err(e) => {
            logging::error(&format!("配置加载失败：{e}；使用默认配置"));
            unsafe {
                MessageBoxW(
                    std::ptr::null_mut(),
                    util::wide(&format!("配置无效：{e}\n已使用默认配置启动。")).as_ptr(),
                    util::wide("WhatTheManDoing").as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
            }
            AppConfig::default()
        }
    };

    let shared = AppShared::new(cfg, config_path);

    // 工作线程：采集 + 上报。
    let worker_shared = shared.clone();
    let worker_handle = thread::Builder::new()
        .name("wtmd-worker".to_string())
        .spawn(move || worker(worker_shared))
        .expect("创建工作线程失败");

    // 托盘。
    if let Err(e) = tray::init(shared.clone()) {
        logging::error(&format!("托盘初始化失败：{e}"));
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                util::wide(&format!("托盘初始化失败：{e}")).as_ptr(),
                util::wide("WhatTheManDoing").as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
        {
            let mut st = shared.lock();
            st.shutdown = true;
        }
        shared.notify();
        let _ = worker_handle.join();
        return;
    }
    tray::request_refresh();

    // 主消息循环（托盘 + 设置窗）。
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    // 收尾：唤醒并等待工作线程（其退出前尽力上报 Stopped）。
    {
        let mut st = shared.lock();
        st.shutdown = true;
        st.capture_running = false;
    }
    shared.notify();
    let _ = worker_handle.join();
    tray::cleanup();
    logging::info("客户端已退出");
}
