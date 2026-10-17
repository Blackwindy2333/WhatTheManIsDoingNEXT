package com.whatthemanddoing.app

import org.json.JSONArray
import org.json.JSONObject

/**
 * 表单 <-> 配置 JSON 映射与字段校验（纯逻辑，JVM 单测覆盖）。
 *
 * 字段与 wtmd-core AppConfig 一一对应；未知字段（如 history_limit、display_name_map）
 * 在 [mergeInto] 中原样保留，避免设置页覆盖 Rust 侧配置。
 */
object ConfigMapper {

    /** 表单聚合（黑名单以逗号分隔的原文承载）。 */
    data class Form(
        val apiBaseUrl: String,
        val deviceId: String,
        val deviceName: String,
        val apiToken: String,
        val pollIntervalMs: String,
        val reportWindowTitle: Boolean,
        val blacklistCsv: String,
        val privacyPause: Boolean
    )

    /** 字段级校验错误（映射到 TextInputLayout 的 error）。 */
    data class FieldErrors(
        val apiBaseUrl: String? = null,
        val deviceId: String? = null,
        val pollIntervalMs: String? = null
    ) {
        val isEmpty: Boolean get() = apiBaseUrl == null && deviceId == null && pollIntervalMs == null
    }

    /** 逗号/中英文逗号/分号分隔 -> 去空白去空项的列表。 */
    fun parseBlacklist(csv: String): List<String> =
        csv.split(',', '，', ';', '；')
            .map { it.trim() }
            .filter { it.isNotEmpty() }

    fun validate(form: Form): FieldErrors {
        val url = form.apiBaseUrl.trim()
        val urlError = when {
            url.isEmpty() -> "服务器地址不能为空"
            !(url.startsWith("http://") || url.startsWith("https://")) -> "必须以 http:// 或 https:// 开头"
            else -> null
        }
        val deviceIdError = if (form.deviceId.trim().isEmpty()) "设备 ID 不能为空" else null
        val pollValue = form.pollIntervalMs.trim().toLongOrNull()
        val pollError = when {
            pollValue == null -> "轮询间隔必须是整数"
            pollValue < 100 -> "轮询间隔必须 ≥ 100 毫秒"
            else -> null
        }
        return FieldErrors(urlError, deviceIdError, pollError)
    }

    /** 从配置 JSON 生成表单。 */
    fun toForm(configJson: String): Form {
        val o = JSONObject(configJson)
        return Form(
            apiBaseUrl = o.optString("api_base_url", ""),
            deviceId = o.optString("device_id", ""),
            deviceName = o.optString("device_name", ""),
            apiToken = o.optString("api_token", ""),
            pollIntervalMs = o.optLong("poll_interval_ms", 1000L).toString(),
            reportWindowTitle = o.optBoolean("report_window_title", false),
            blacklistCsv = o.optJSONArray("app_name_blacklist")
                ?.let { arr -> (0 until arr.length()).joinToString(", ") { arr.getString(it) } }
                ?: "",
            privacyPause = o.optBoolean("privacy_pause", false)
        )
    }

    /**
     * 把表单合并进既有配置 JSON（保留 history_limit、display_name_map 等未暴露字段）。
     * 调用前须 [validate] 通过。
     */
    fun mergeInto(existingConfigJson: String, form: Form): String {
        val o = JSONObject(existingConfigJson)
        o.put("api_base_url", form.apiBaseUrl.trim())
        o.put("device_id", form.deviceId.trim())
        o.put("device_name", form.deviceName.trim())
        o.put("api_token", form.apiToken)
        o.put("poll_interval_ms", form.pollIntervalMs.trim().toLongOrNull() ?: 1000L)
        o.put("report_window_title", form.reportWindowTitle)
        o.put("privacy_pause", form.privacyPause)
        val arr = JSONArray()
        parseBlacklist(form.blacklistCsv).forEach { arr.put(it) }
        o.put("app_name_blacklist", arr)
        return o.toString()
    }
}
