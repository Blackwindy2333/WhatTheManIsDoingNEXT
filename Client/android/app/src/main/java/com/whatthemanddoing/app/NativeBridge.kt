package com.whatthemanddoing.app

import android.content.Context
import java.io.File

/**
 * JNI 门面：所有业务逻辑在 Rust 核心（wtmd-core）与桥 crate（wtmd-android-bridge）中。
 *
 * 导出符号见 Client/android/rust/src/lib.rs：
 *   Java_com_whatthemanddoing_app_NativeBridge_*（object 成员 @JvmStatic 对应的稳定签名）。
 */
object NativeBridge {
    init {
        System.loadLibrary("wtmd_android_bridge")
    }

    @Volatile
    private var initialized = false

    /**
     * 兜底初始化：任何入口（无障碍服务、前台服务、开机广播）先调一次。
     * Rust 侧 init() 幂等；没有它，开机自启/系统拉活路径下事件会被静默丢弃。
     */
    @JvmStatic
    fun ensureInitialized(context: Context): Boolean {
        if (initialized) return true
        synchronized(this) {
            if (!initialized) {
                initialized = initBridge(File(context.filesDir, "config.json").absolutePath)
            }
            return initialized
        }
    }

    /** 加载（或创建）配置并启动上报线程。幂等。返回 false=配置加载出错（已回退默认值）或启动失败。 */
    @JvmStatic
    external fun initBridge(configPath: String): Boolean

    /** 前台应用切换（包名 + 窗口类名）；Rust 侧按包名去重节流（~300ms）。 */
    @JvmStatic
    external fun onForegroundChange(packageName: String, className: String)

    /** 隐私暂停开关：true=暂停上报（app=null），false=恢复；写入配置并持久化。 */
    @JvmStatic
    external fun setPaused(paused: Boolean)

    /** 停止采集并尽力上报 stopped。 */
    @JvmStatic
    external fun setStopped()

    /** 通知栏状态 JSON；JVM 异常时不返回 null 而是空串，仍声明可空以对 string_to_jstring 失败兜底。 */
    @JvmStatic
    external fun getStatusJson(): String?

    /** 配置 JSON（供设置页回填）。 */
    @JvmStatic
    external fun getConfigJson(): String?

    /** 保存配置 JSON：validate + 持久化 + 热应用。返回 false=校验或保存失败。 */
    @JvmStatic
    external fun saveConfigJson(json: String): Boolean
}
