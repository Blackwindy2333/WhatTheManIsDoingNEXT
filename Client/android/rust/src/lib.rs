//! wtmd-android-bridge：JNI 导出层（Kotlin 薄壳 ↔ wtmd-core）。
//!
//! 约定：
//! - 业务逻辑全部在 wtmd-core 与 `service`/`state` 模块，本层只做
//!   字符串/布尔转换与 panic 隔离；
//! - 任何 Rust panic 都不跨 FFI（`catch_unwind` 兜底为失败/空值）；
//! - JNI 第二参数是 Kotlin `object NativeBridge` 的实例引用（未使用）。
//!
//! 对应 Kotlin：`com.whatthemanddoing.app.NativeBridge`。

mod service;
mod state;

use std::panic::{catch_unwind, AssertUnwindSafe};

use jni::objects::{JObject, JString};
use jni::sys::{jboolean, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;

/// 执行闭包并吞掉 panic（返回 fallback），保证不跨 FFI 展开。
fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

fn jstring_to_string(env: &mut JNIEnv, s: &JString) -> String {
    match env.get_string(s) {
        Ok(js) => js.into(),
        Err(_) => String::new(),
    }
}

fn string_to_jstring<'local>(env: &mut JNIEnv<'local>, s: &str) -> JString<'local> {
    match env.new_string(s) {
        Ok(js) => js,
        Err(_) => JString::from(JObject::null()),
    }
}

fn bool_to_jboolean(b: bool) -> jboolean {
    if b {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

/// 初始化：load_or_create 配置 + 启动上报线程（幂等）。
/// 返回 false 表示配置加载有误（回退默认）或上报线程启动失败。
#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn Java_com_whatthemanddoing_app_NativeBridge_initBridge<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    config_path: JString<'local>,
) -> jboolean {
    let path = jstring_to_string(&mut env, &config_path);
    bool_to_jboolean(guard(false, || service::init(&path)))
}

/// 前台应用变化（包名 + Activity 类名）；桥内按包名去重节流。
#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn Java_com_whatthemanddoing_app_NativeBridge_onForegroundChange<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    package_name: JString<'local>,
    class_name: JString<'local>,
) {
    let pkg = jstring_to_string(&mut env, &package_name);
    let cls = jstring_to_string(&mut env, &class_name);
    guard((), || service::on_foreground_change(&pkg, &cls));
}

/// 隐私暂停开/关：持久化配置并上报状态切换。
#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn Java_com_whatthemanddoing_app_NativeBridge_setPaused<'local>(
    _env: JNIEnv<'local>,
    _this: JObject<'local>,
    paused: jboolean,
) {
    let paused = paused == JNI_TRUE;
    guard((), || service::set_paused(paused));
}

/// 停止采集：上报 stopped（app=None），此后忽略前台事件。
#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn Java_com_whatthemanddoing_app_NativeBridge_setStopped<'local>(
    _env: JNIEnv<'local>,
    _this: JObject<'local>,
) {
    guard((), service::set_stopped);
}

/// 状态 JSON（通知栏/状态区用）。
#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn Java_com_whatthemanddoing_app_NativeBridge_getStatusJson<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
) -> JString<'local> {
    let json = guard(String::new(), service::get_status_json);
    string_to_jstring(&mut env, &json)
}

/// 当前配置 JSON（设置界面读取）。
#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn Java_com_whatthemanddoing_app_NativeBridge_getConfigJson<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
) -> JString<'local> {
    let json = guard(String::new(), service::get_config_json);
    string_to_jstring(&mut env, &json)
}

/// 保存配置 JSON：解析 + 校验 + 持久化 + 热应用。成功 true。
#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn Java_com_whatthemanddoing_app_NativeBridge_saveConfigJson<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    json: JString<'local>,
) -> jboolean {
    let json = jstring_to_string(&mut env, &json);
    bool_to_jboolean(guard(false, || service::save_config_json(&json)))
}
