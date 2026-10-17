package com.whatthemanddoing.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class StatusFormatterTest {

    private val fullJson = """
        {"initialized":true,"state":"active","privacy_pause":false,"report_window_title":false,
         "current_app":{"process_name":"com.tencent.mm","display_name":"微信","window_title":null},
         "last_report":{"outcome":"sent","at":"2026-01-01T00:00:00Z","error":null},
         "last_error":null,"queue_len":0,"api_base_url":"http://127.0.0.1:8080","device_id":"my-device"}
    """.trimIndent()

    @Test
    fun `notification text contains state app and report`() {
        val text = StatusFormatter.notificationText(fullJson)
        assertTrue(text.contains("active"))
        assertTrue(text.contains("微信"))
        assertTrue(text.contains("sent"))
    }

    @Test
    fun `current app prefers display name over process name`() {
        assertEquals("微信", StatusFormatter.currentAppLabel(fullJson))
    }

    @Test
    fun `current app falls back to process name and then empty marker`() {
        val noDisplay = """{"current_app":{"process_name":"com.a","display_name":null}}"""
        assertEquals("com.a", StatusFormatter.currentAppLabel(noDisplay))
        val noApp = """{"current_app":null}"""
        assertEquals("（无）", StatusFormatter.currentAppLabel(noApp))
    }

    @Test
    fun `last report shows error detail when present`() {
        val json = """{"last_report":{"outcome":"network_error","at":"2026-01-01T00:00:00Z","error":"connection refused"}}"""
        val label = StatusFormatter.lastReportLabel(json)
        assertTrue(label.contains("network_error"))
        assertTrue(label.contains("connection refused"))
    }

    @Test
    fun `last report missing shows pending marker`() {
        assertEquals("（尚未上报）", StatusFormatter.lastReportLabel("{}"))
    }

    @Test
    fun `paused flag reads privacy_pause`() {
        assertTrue(StatusFormatter.isPaused("""{"privacy_pause":true}"""))
        assertFalse(StatusFormatter.isPaused(fullJson))
    }

    @Test
    fun `json null fields must not render as literal null`() {
        // Android 的 org.json 对 JSON null 会让 optString 返回字面量 "null"（冒烟实测症状），
        // 必须走 null 安全读取；本用例在 JSON-java 与 Android org.json 下都要成立。
        val json = """{"last_report":{"outcome":"sent","at":"2026-10-17T09:44:45Z","error":null}}"""
        val label = StatusFormatter.lastReportLabel(json)
        assertEquals("sent（2026-10-17T09:44:45Z）", label)
        assertFalse(label.contains("null"))

        val noErrorField = """{"last_report":{"outcome":"sent","at":"T1"}}"""
        assertFalse(StatusFormatter.lastReportLabel(noErrorField).contains("null"))

        val nullNames = """{"current_app":{"process_name":"com.a","display_name":null,"window_title":null}}"""
        assertEquals("com.a", StatusFormatter.currentAppLabel(nullNames))
    }

    @Test
    fun `malformed json degrades gracefully`() {
        assertEquals("状态未知", StatusFormatter.notificationText("not json"))
        assertEquals("（无）", StatusFormatter.currentAppLabel("not json"))
    }
}
