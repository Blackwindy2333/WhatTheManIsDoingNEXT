package com.whatthemanddoing.app

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat

/**
 * 开机引导：无障碍服务启用态由系统跨重启持久化并自动拉活（无需代码）；
 * 本接收器只负责把控制面常驻通知重新拉起，未启用则发引导通知（不产生假数据）。
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED) return

        if (MainActivity.isAccessibilityEnabled(context)) {
            val serviceIntent = Intent(context, ForegroundService::class.java)
            try {
                context.startForegroundService(serviceIntent)
            } catch (_: Exception) {
                // 厂商限制后台启动时用户仍可手动打开应用
            }
        } else {
            notifyGuide(context)
        }
    }

    private fun notifyGuide(context: Context) {
        try {
            val openApp = PendingIntent.getActivity(
                context, 0, Intent(context, MainActivity::class.java),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
            )
            val notification = NotificationCompat.Builder(context, "wtmd_service")
                .setSmallIcon(R.drawable.ic_notification)
                .setContentTitle(context.getString(R.string.app_name))
                .setContentText(context.getString(R.string.notify_accessibility_needed))
                .setContentIntent(openApp)
                .setAutoCancel(true)
                .build()
            NotificationManagerCompat.from(context).notify(31, notification)
        } catch (_: Exception) {
            // 未授予 POST_NOTIFICATIONS 时静默
        }
    }
}
