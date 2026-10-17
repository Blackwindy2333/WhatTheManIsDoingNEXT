/**
 * WhatTheManDoing — public read-only monitor (static front-end).
 * All requests are same-origin: /api/public/* only; device tokens never reach the browser.
 * Pure static HTML/CSS/JS — no build step.
 */

const state = {
  publicConfig: {
    page_title: "在干什么",
    page_subtitle: "What The Man Doing",
    refresh_interval_seconds: 5,
    show_history: true,
  },
  devices: [],
  selectedId: null,
  timer: null,
  eventSource: null,
};

const el = {
  pageTitle: document.getElementById("page-title"),
  pageSubtitle: document.getElementById("page-subtitle"),
  connPill: document.getElementById("conn-pill"),
  connLabel: document.getElementById("conn-label"),
  heroMeta: document.getElementById("hero-meta"),
  statOnline: document.getElementById("stat-online"),
  statTotal: document.getElementById("stat-total"),
  deviceGrid: document.getElementById("device-grid"),
  deviceCount: document.getElementById("device-count"),
  devicesEmpty: document.getElementById("devices-empty"),
  devicesError: document.getElementById("devices-error"),
  devicesErrorText: document.getElementById("devices-error-text"),
  historySection: document.getElementById("history-section"),
  historyDeviceLabel: document.getElementById("history-device-label"),
  timeline: document.getElementById("timeline"),
  historyEmpty: document.getElementById("history-empty"),
};

function setConn(mode, label) {
  el.connPill.dataset.state = mode;
  el.connLabel.textContent = label;
}

async function api(path, options = {}) {
  const res = await fetch(path, {
    ...options,
    headers: {
      ...(options.body ? { "Content-Type": "application/json" } : {}),
      Accept: "application/json",
      ...(options.headers || {}),
    },
  });
  let body = null;
  try {
    body = await res.json();
  } catch {
    body = { code: -1, message: `HTTP ${res.status}`, data: null };
  }
  return { res, body };
}

function formatTime(iso) {
  if (!iso) return "—";
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

function formatUpdated(epochSeconds) {
  if (!epochSeconds) return "—";
  return new Date(epochSeconds * 1000).toLocaleTimeString();
}

function statusBadge(device) {
  if (device.enabled === false) {
    return { cls: "warn", label: "已停用" };
  }
  if (device.status === "paused") {
    return { cls: "warn", label: "已暂停" };
  }
  if (device.status === "stopped") {
    return { cls: "danger", label: "已停止" };
  }
  if (device.online && device.healthy !== false && !device.error) {
    return { cls: "ok", label: "在线" };
  }
  return { cls: "danger", label: device.error ? "异常" : "离线" };
}

function appLabel(device) {
  const app = device.app;
  if (!app) {
    if (device.status === "paused") return "已暂停分享";
    return "无前台应用";
  }
  return app.display_name || app.process_name || "未知应用";
}

function renderPublicConfig() {
  el.pageTitle.textContent = state.publicConfig.page_title || "在干什么";
  el.pageSubtitle.textContent = state.publicConfig.page_subtitle || "";
  document.title = state.publicConfig.page_title || "在干什么";
  el.historySection.hidden = !state.publicConfig.show_history;
}

function renderDevices() {
  const devices = state.devices || [];
  el.deviceGrid.innerHTML = "";
  const online = devices.filter((d) => d.online && d.enabled !== false).length;
  el.statOnline.textContent = String(online);
  el.statTotal.textContent = String(devices.length);
  el.deviceCount.textContent = devices.length ? `${devices.length} 台` : "";
  el.heroMeta.textContent = devices.length
    ? `${online} / ${devices.length} 在线`
    : "暂无设备上报";

  if (!devices.length) {
    el.devicesEmpty.hidden = false;
    el.devicesError.hidden = true;
    return;
  }
  el.devicesEmpty.hidden = true;

  const fragment = document.createDocumentFragment();
  for (const device of devices) {
    const badge = statusBadge(device);
    const card = document.createElement("article");
    card.className = "device-card" + (state.selectedId === device.id ? " is-selected" : "");
    card.tabIndex = 0;
    card.dataset.id = device.id;
    card.innerHTML = `
      <div class="device-card-top">
        <div>
          <h3 class="device-name"></h3>
          <p class="device-id"></p>
        </div>
        <span class="badge ${badge.cls}"><span class="dot"></span><span class="badge-label"></span></span>
      </div>
      <p class="app-name"></p>
      <p class="app-meta"></p>
      <div class="health-row">
        <span>延迟 <strong class="latency"></strong></span>
        <span>HTTP <strong class="http"></strong></span>
        <span>更新 <strong class="updated"></strong></span>
      </div>
    `;
    card.querySelector(".device-name").textContent = device.name || device.id;
    card.querySelector(".device-id").textContent = device.id;
    card.querySelector(".badge-label").textContent = badge.label;
    card.querySelector(".app-name").textContent = appLabel(device);
    const meta = device.app?.window_title || device.timestamp;
    card.querySelector(".app-meta").textContent = meta
      ? (device.app?.window_title ? `标题：${device.app.window_title}` : `时间：${formatTime(device.timestamp)}`)
      : "";
    card.querySelector(".latency").textContent =
      device.latency_ms != null ? `${device.latency_ms}ms` : "—";
    card.querySelector(".http").textContent = device.http_status != null ? String(device.http_status) : "—";
    card.querySelector(".updated").textContent = formatUpdated(device.updated_at);
    card.addEventListener("click", () => selectDevice(device.id));
    card.addEventListener("keydown", (e) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        selectDevice(device.id);
      }
    });
    fragment.appendChild(card);
  }
  el.deviceGrid.appendChild(fragment);
}

async function loadPublicConfig() {
  const { body } = await api("/api/public/config");
  if (body?.code === 0 && body.data) {
    state.publicConfig = { ...state.publicConfig, ...body.data };
    renderPublicConfig();
  }
}

/** Reachability diagnostic via the same-origin probe endpoint. */
async function probeHint() {
  try {
    const { body } = await api("/api/public/probe");
    if (body?.code === 0 && body.data) {
      const d = body.data;
      const where = [d.scheme, d.host].filter(Boolean).join("://");
      return `（服务可达，诊断 ${d.method || "GET"} ${d.path || "/api/public/probe"}${where ? " · " + where : ""}）`;
    }
  } catch {
    /* probe unreachable */
  }
  return "（诊断接口也不可达，请确认服务已启动）";
}

async function showDevicesError(message) {
  el.devicesError.hidden = false;
  el.devicesErrorText.textContent = `${message}${await probeHint()}`;
  setConn("offline", "异常");
}

async function loadDevices() {
  let body;
  try {
    ({ body } = await api("/api/public/devices"));
  } catch {
    await showDevicesError("无法连接到服务，请稍后重试。");
    return;
  }
  if (body?.code !== 0) {
    await showDevicesError(body?.message || "服务返回错误");
    return;
  }
  el.devicesError.hidden = true;
  state.devices = body.data?.devices || [];
  renderDevices();
  setConn("online", "已连接");
}

function selectDevice(id) {
  state.selectedId = state.selectedId === id ? null : id;
  renderDevices();
  if (state.selectedId && state.publicConfig.show_history) {
    loadHistory(state.selectedId);
  } else {
    el.timeline.innerHTML = "";
    el.historyEmpty.hidden = false;
    el.historyEmpty.textContent = "选择一台设备后显示时间线。";
    el.historyDeviceLabel.textContent = "选择一台设备查看";
  }
}

async function loadHistory(deviceId) {
  el.historyDeviceLabel.textContent = deviceId;
  el.historyEmpty.hidden = true;
  el.timeline.innerHTML = `<li class="empty">加载中…</li>`;
  const { body } = await api(`/api/public/devices/${encodeURIComponent(deviceId)}/history?limit=20`);
  if (body?.code !== 0) {
    el.timeline.innerHTML = "";
    el.historyEmpty.hidden = false;
    el.historyEmpty.textContent = body?.message || "无法加载历史";
    return;
  }
  const history = body.data?.history || [];
  if (!history.length) {
    el.timeline.innerHTML = "";
    el.historyEmpty.hidden = false;
    el.historyEmpty.textContent = "暂无历史记录";
    return;
  }
  el.historyEmpty.hidden = true;
  el.timeline.innerHTML = "";
  for (const item of history) {
    const li = document.createElement("li");
    const left = document.createElement("span");
    left.className = "t-app";
    left.textContent = item.display_name || item.process_name || item.status || "—";
    const right = document.createElement("span");
    right.className = "t-time";
    right.textContent = formatTime(item.timestamp);
    li.append(left, right);
    el.timeline.appendChild(li);
  }
}

function stopStream() {
  if (state.eventSource) {
    state.eventSource.close();
    state.eventSource = null;
  }
}

function startPolling() {
  stopStream();
  if (state.timer) clearInterval(state.timer);
  const ms = Math.max(1, state.publicConfig.refresh_interval_seconds || 5) * 1000;
  state.timer = setInterval(() => {
    loadDevices().catch(() => {});
  }, ms);
  loadDevices().catch(() => {});
}

function startStream() {
  stopStream();
  if (!window.EventSource) {
    startPolling();
    return;
  }
  const es = new EventSource("/api/public/stream");
  state.eventSource = es;
  es.onmessage = (ev) => {
    try {
      const msg = JSON.parse(ev.data);
      if (msg.type === "devices" && Array.isArray(msg.data)) {
        state.devices = msg.data;
        renderDevices();
        setConn("online", "实时");
      }
    } catch {
      /* ignore malformed frame */
    }
  };
  es.onerror = () => {
    es.close();
    state.eventSource = null;
    setConn("connecting", "重连中…");
    startPolling();
  };
}

async function boot() {
  renderPublicConfig(); // apply defaults first so the skeleton renders even if config fetch fails
  await loadPublicConfig();
  startStream();
  // Fallback refresh also keeps history-less pages feeling live if SSE is blocked
  if (state.timer) clearInterval(state.timer);
  state.timer = setInterval(() => {
    if (!state.eventSource) loadDevices().catch(() => {});
  }, Math.max(3, state.publicConfig.refresh_interval_seconds || 5) * 1000);
}

boot().catch((err) => {
  setConn("offline", "启动失败");
  console.error(err);
});
