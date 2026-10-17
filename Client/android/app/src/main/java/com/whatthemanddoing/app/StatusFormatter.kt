package com.whatthemanddoing.app

import org.json.JSONObject

/**
 * get_status_json() -> 通知栏/状态区文本（纯逻辑，JVM 单测覆盖）。
 *
 * JSON 形态（由 Rust 侧 status_json 产出）：
 * {initialized, state, privacy_pause, report_window_title,
 *  current_app:{process_name,display_name,window_title}|null,
 *  last_report:{outcome,at,error}|null, last_error, queue_len,
 *  api_base_url, device_id}
 */
object StatusFormatter {

    /** 常驻通知正文（多行，简洁）。 */
    fun notificationText(statusJson: String): String {
        val o = parse(statusJson) ?: return "状态未知"
        val app = currentAppLabel(o)
        val report = lastReportLabel(o)
        val state = str(o, "state").ifEmpty { "?" }
        return "状态：$state\n应用：$app\n上报：$report"
    }

    /** 状态区单行：当前应用。 */
    fun currentAppLabel(statusJson: String): String =
        parse(statusJson)?.let(::currentAppLabel) ?: "（无）"

    /** 状态区单行：最近上报。 */
    fun lastReportLabel(statusJson: String): String =
        parse(statusJson)?.let(::lastReportLabel) ?: "（无）"

    /** 通知动作按钮是否应显示"暂停"（当前未暂停）。 */
    fun isPaused(statusJson: String): Boolean =
        parse(statusJson)?.let { it.optBoolean("privacy_pause", false) } ?: false

    private fun parse(statusJson: String): JSONObject? = try {
        JSONObject(statusJson)
    } catch (_: Exception) {
        null
    }

    private fun currentAppLabel(o: JSONObject): String {
        val app = o.optJSONObject("current_app") ?: return "（无）"
        val name = str(app, "display_name").ifEmpty { str(app, "process_name") }
        return name.ifEmpty { "（无）" }
    }

    private fun lastReportLabel(o: JSONObject): String {
        val r = o.optJSONObject("last_report") ?: return "（尚未上报）"
        val outcome = str(r, "outcome").ifEmpty { "?" }
        val at = str(r, "at")
        val err = str(r, "error").takeIf { it.isNotEmpty() }
        return if (err != null) "$outcome（$at）$err" else "$outcome（$at）"
    }

    /**
     * null 安全取字符串：缺失或 JSON null 都返回空串。
     * 注意 Android 的 org.json 对 JSON null 会让 optString 返回字面量 "null"，
     * 因此统一走 opt + JSONObject.NULL 判断。
     */
    private fun str(o: JSONObject, key: String): String {
        val v = o.opt(key) ?: return ""
        if (v == JSONObject.NULL) return ""
        return v.toString()
    }
}
