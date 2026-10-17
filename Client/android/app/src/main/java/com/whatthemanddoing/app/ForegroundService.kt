package com.whatthemanddoing.app

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat

/**
 * 控制面前台服务（specialUse + 常驻通知）。
 *
 * 职责：常驻通知展示 get_status_json；动作按钮=暂停/恢复上报、停止采集。
 * 实际采集在 AccessibilityService 事件回调 + Rust 上报线程中完成，本服务只是外壳与生命周期锚点。
 */
class ForegroundService : Service() {

    private val handler = Handler(Looper.getMainLooper())
    private var lastStatusText: String? = null

    private val refreshRunnable = object : Runnable {
        override fun run() {
            updateNotification()
            handler.postDelayed(this, 3_000L)
        }
    }

    override fun onCreate() {
        super.onCreate()
        // 开机/系统拉活路径下 MainActivity 可能从未运行过：兜底初始化桥（幂等）
        NativeBridge.ensureInitialized(this)
        createChannel()
        startAsForeground()
        handler.post(refreshRunnable)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_TOGGLE_PAUSE -> {
                val paused = try {
                    StatusFormatter.isPaused(NativeBridge.getStatusJson() ?: "")
                } catch (_: Throwable) {
                    false
                }
                try {
                    NativeBridge.setPaused(!paused)
                } catch (_: Throwable) {
                }
                updateNotification()
            }
            ACTION_STOP -> {
                try {
                    NativeBridge.setStopped()
                } catch (_: Throwable) {
                }
                handler.postDelayed({ stopSelf() }, 1_500L) // 留宽限让 stopped 报文发出
            }
        }
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        handler.removeCallbacks(refreshRunnable)
        try {
            NativeBridge.setStopped() // 进程退出前尽力上报 stopped
        } catch (_: Throwable) {
        }
        super.onDestroy()
    }

    override fun onTaskRemoved(rootIntent: Intent?) {
        try {
            NativeBridge.setStopped()
        } catch (_: Throwable) {
        }
        super.onTaskRemoved(rootIntent)
    }

    private fun createChannel() {
        if (Build.VERSION.SDK_INT >= 26) {
            val manager = getSystemService(NotificationManager::class.java) ?: return
            manager.createNotificationChannel(
                NotificationChannel(
                    CHANNEL_ID,
                    getString(R.string.channel_status_name),
                    NotificationManager.IMPORTANCE_LOW
                )
            )
        }
    }

    private fun startAsForeground() {
        val notification = buildNotification()
        if (Build.VERSION.SDK_INT >= 29) {
            ServiceCompat.startForeground(
                this, NOTIFICATION_ID, notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    private fun buildNotification(): android.app.Notification {
        val statusText = try {
            StatusFormatter.notificationText(NativeBridge.getStatusJson() ?: "")
        } catch (_: Throwable) {
            getString(R.string.status_value_unknown)
        }
        val paused = try {
            StatusFormatter.isPaused(NativeBridge.getStatusJson() ?: "")
        } catch (_: Throwable) {
            false
        }

        val openApp = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
        val togglePause = PendingIntent.getService(
            this, 1,
            Intent(this, ForegroundService::class.java).setAction(ACTION_TOGGLE_PAUSE),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
        val stop = PendingIntent.getService(
            this, 2,
            Intent(this, ForegroundService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        val builder = NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(getString(R.string.notify_status_title))
            .setContentText(statusText)
            .setStyle(NotificationCompat.BigTextStyle().bigText(statusText))
            .setContentIntent(openApp)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .addAction(
                0,
                getString(if (paused) R.string.notify_action_resume else R.string.notify_action_pause),
                togglePause
            )
            .addAction(0, getString(R.string.notify_action_stop), stop)
        return builder.build()
    }

    private fun updateNotification() {
        val statusText = try {
            StatusFormatter.notificationText(NativeBridge.getStatusJson() ?: "")
        } catch (_: Throwable) {
            getString(R.string.status_value_unknown)
        }
        if (statusText == lastStatusText) return
        lastStatusText = statusText
        val manager = getSystemService(NotificationManager::class.java) ?: return
        manager.notify(NOTIFICATION_ID, buildNotification())
    }

    companion object {
        const val ACTION_TOGGLE_PAUSE = "com.whatthemanddoing.app.TOGGLE_PAUSE"
        const val ACTION_STOP = "com.whatthemanddoing.app.STOP_COLLECT"
        private const val CHANNEL_ID = "wtmd_status"
        private const val NOTIFICATION_ID = 21
    }
}
