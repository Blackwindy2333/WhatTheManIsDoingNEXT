package com.whatthemanddoing.app

/**
 * 前台切换事件节流（纯逻辑，可注入时钟以便 JVM 单测）。
 *
 * 规则：同一 (packageName, className) 在 [minIntervalMs] 内重复出现视为重复，应丢弃；
 * 跨包/跨类名立即放行（前台切换必须即时上报）。与无障碍 notificationTimeout=300ms
 * 及 Rust 侧包名去重节流叠加。
 */
class EventThrottle(
    private val minIntervalMs: Long = 300L,
    private val clock: () -> Long = System::currentTimeMillis
) {
    private var lastKey: String? = null
    private var lastAtMs: Long = 0L

    /** 可见性：internal 供单测直接观察；Kotlin 单测在同一模块内。 */
    internal var acceptedCount: Int = 0
        private set

    /** @return true=该事件应放行并上报 */
    fun shouldAccept(packageName: String, className: String): Boolean {
        val key = packageName.trim() + "|" + className.trim()
        val now = clock()
        val accepted = if (key == lastKey && now - lastAtMs < minIntervalMs) {
            false
        } else {
            true
        }
        if (accepted) {
            lastKey = key
            lastAtMs = now
            acceptedCount++
        }
        return accepted
    }
}
