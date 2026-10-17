package com.whatthemanddoing.app

import android.accessibilityservice.AccessibilityService
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Intent
import android.os.Build
import android.view.accessibility.AccessibilityEvent
import androidx.core.app.NotificationCompat

/**
 * 前台应用监听（无障碍服务）。
 *
 * 只读 event.packageName / event.className（TYPE_WINDOW_STATE_CHANGED，经
 * res/xml/accessibility_service_config.xml 限定），不读窗口内容（隐私）。
 * 系统 bindService 绑定：进程被杀自动重启、启用态跨重启保持（无需代码保活）。
 */
class ForegroundWatchService : AccessibilityService() {

    private val throttle = EventThrottle(minIntervalMs = 300L)

    override fun onServiceConnected() {
        super.onServiceConnected()
        // 开机/系统拉活路径下 MainActivity 可能从未运行过：兜底初始化桥
        NativeBridge.ensureInitialized(this)
        // 拉起控制面常驻通知服务（状态展示 + 暂停/停止控制）
        try {
            if (Build.VERSION.SDK_INT >= 26) {
                startForegroundService(Intent(this, ForegroundService::class.java))
            }
        } catch (_: Exception) {
            // 通知渠道/权限问题不阻塞监听
        }
        notifyLight(getString(R.string.notify_service_enabled))
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        val pkg = event?.packageName?.toString() ?: return
        if (event.eventType != AccessibilityEvent.TYPE_WINDOW_STATE_CHANGED) return
        val cls = event.className?.toString().orEmpty()
        if (!throttle.shouldAccept(pkg, cls)) return
        try {
            NativeBridge.onForegroundChange(pkg, cls)
        } catch (_: Throwable) {
            // 不让 JNI 异常杀死无障碍服务
        }
    }

    override fun onInterrupt() {
        // 无障碍服务被系统中断：无状态需清理（上报线程在 Rust 侧继续）
    }

    override fun onDestroy() {
        // 注意：此处不调用 setStopped()。无障碍服务的 onDestroy 可能由系统瞬时回收
        // 触发（如 uiautomator/其他无障碍客户端抢占连接、系统重绑定），进程并未退出；
        // 而 Rust 侧 stopped 状态是粘性的（仅 set_paused/init 复位），误报 stopped 会
        // 使后续前台切换全部丢弃。进程退出前上报 stopped 由 ForegroundService
        // 的 onDestroy/onTaskRemoved 负责（控制面语义）。
        notifyLight(getString(R.string.notify_service_disabled))
        super.onDestroy()
    }

    /** 服务启停轻量通知（与常驻通知分渠道）。 */
    private fun notifyLight(text: String) {
        val manager = getSystemService(NotificationManager::class.java) ?: return
        if (Build.VERSION.SDK_INT >= 26) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                getString(R.string.channel_service_name),
                NotificationManager.IMPORTANCE_LOW
            )
            manager.createNotificationChannel(channel)
        }
        val intent = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
        val notification = NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(text)
            .setContentIntent(intent)
            .setAutoCancel(true)
            .build()
        manager.notify(TAG, NOTIFICATION_ID, notification)
    }

    companion object {
        private const val CHANNEL_ID = "wtmd_service"
        private const val TAG = "watch_service"
        private const val NOTIFICATION_ID = 11
    }
}
