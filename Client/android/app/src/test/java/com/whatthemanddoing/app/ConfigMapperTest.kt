package com.whatthemanddoing.app

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ConfigMapperTest {

    private fun form(
        url: String = "http://127.0.0.1:8080",
        deviceId: String = "phone-1",
        poll: String = "1000",
        reportTitle: Boolean = false,
        blacklist: String = "",
        privacyPause: Boolean = false
    ) = ConfigMapper.Form(
        apiBaseUrl = url,
        deviceId = deviceId,
        deviceName = "My Device",
        apiToken = "token",
        pollIntervalMs = poll,
        reportWindowTitle = reportTitle,
        blacklistCsv = blacklist,
        privacyPause = privacyPause
    )

    // ---- 校验映射 ----

    @Test
    fun `valid form passes validation`() {
        assertTrue(ConfigMapper.validate(form()).isEmpty)
    }

    @Test
    fun `url must use http or https`() {
        val errors = ConfigMapper.validate(form(url = "ftp://x"))
        assertFalse(errors.isEmpty)
        assertEquals("必须以 http:// 或 https:// 开头", errors.apiBaseUrl)
        assertNull(errors.deviceId)
    }

    @Test
    fun `url cannot be blank`() {
        assertEquals("服务器地址不能为空", ConfigMapper.validate(form(url = "  ")).apiBaseUrl)
    }

    @Test
    fun `device id must not be blank`() {
        assertEquals("设备 ID 不能为空", ConfigMapper.validate(form(deviceId = " ")).deviceId)
    }

    @Test
    fun `poll interval must be at least 100`() {
        assertEquals("轮询间隔必须 ≥ 100 毫秒", ConfigMapper.validate(form(poll = "99")).pollIntervalMs)
        assertNull(ConfigMapper.validate(form(poll = "100")).pollIntervalMs)
    }

    @Test
    fun `poll interval must be an integer`() {
        assertEquals("轮询间隔必须是整数", ConfigMapper.validate(form(poll = "abc")).pollIntervalMs)
    }

    // ---- 黑名单 CSV ----

    @Test
    fun `blacklist parses mixed separators and trims blanks`() {
        assertEquals(
            listOf("com.game", "com.tiktok", "抖音"),
            ConfigMapper.parseBlacklist(" com.game ， com.tiktok;;, 抖音 ")
        )
    }

    // ---- 表单 <-> JSON ----

    @Test
    fun `mergeInto maps all form fields and keeps unknown fields`() {
        val existing = """{"api_base_url":"http://old","device_id":"old","device_name":"old","api_token":"old","poll_interval_ms":5000,"report_window_title":false,"app_name_blacklist":[],"privacy_pause":false,"history_limit":500,"display_name_map":{"com.a":"A"}}"""
        val merged = ConfigMapper.mergeInto(
            existing,
            form(url = "https://server:8443", deviceId = "dev-2", poll = "2500", reportTitle = true, blacklist = "com.game,com.news", privacyPause = true)
        )
        val o = JSONObject(merged)
        assertEquals("https://server:8443", o.getString("api_base_url"))
        assertEquals("dev-2", o.getString("device_id"))
        assertEquals(2500L, o.getLong("poll_interval_ms"))
        assertTrue(o.getBoolean("report_window_title"))
        assertTrue(o.getBoolean("privacy_pause"))
        val arr = o.getJSONArray("app_name_blacklist")
        assertEquals(2, arr.length())
        assertEquals("com.game", arr.getString(0))
        // 未暴露字段保持原值（不被设置页覆盖）
        assertEquals(500, o.getInt("history_limit"))
        assertEquals("A", o.getJSONObject("display_name_map").getString("com.a"))
    }

    @Test
    fun `toForm round-trips through mergeInto`() {
        val original = """{"api_base_url":"http://127.0.0.1:8080","device_id":"my-device","device_name":"My Device","api_token":"change-me","poll_interval_ms":1000,"report_window_title":true,"app_name_blacklist":["com.game"],"privacy_pause":true,"history_limit":500,"display_name_map":{}}"""
        val form = ConfigMapper.toForm(original)
        val merged = ConfigMapper.mergeInto(original, form)
        val a = JSONObject(original)
        val b = JSONObject(merged)
        assertEquals(a.getString("api_base_url"), b.getString("api_base_url"))
        assertEquals(a.getString("device_id"), b.getString("device_id"))
        assertEquals(a.getLong("poll_interval_ms"), b.getLong("poll_interval_ms"))
        assertEquals(a.getBoolean("report_window_title"), b.getBoolean("report_window_title"))
        assertEquals(a.getBoolean("privacy_pause"), b.getBoolean("privacy_pause"))
        assertEquals(
            a.getJSONArray("app_name_blacklist").getString(0),
            b.getJSONArray("app_name_blacklist").getString(0)
        )
    }

    @Test
    fun `toForm tolerates missing fields via defaults`() {
        val form = ConfigMapper.toForm("{}")
        assertEquals("", form.apiBaseUrl)
        assertEquals("1000", form.pollIntervalMs)
        assertFalse(form.reportWindowTitle)
        assertFalse(form.privacyPause)
    }
}
