package com.whatthemanddoing.app

import android.Manifest
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.widget.TextView
import android.widget.Toast
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import com.google.android.material.button.MaterialButton
import com.google.android.material.materialswitch.MaterialSwitch
import com.google.android.material.textfield.TextInputEditText
import com.google.android.material.textfield.TextInputLayout
import java.io.File

/**
 * 设置页（Material 3，深浅色跟随系统）。
 * 表单读写全部经 NativeBridge（JNI -> wtmd-core），Kotlin 不做业务规则。
 */
class MainActivity : AppCompatActivity() {

    private lateinit var tvServiceStatus: TextView
    private lateinit var tvCurrentApp: TextView
    private lateinit var tvLastReport: TextView

    private lateinit var tilApiBaseUrl: TextInputLayout
    private lateinit var tilDeviceId: TextInputLayout
    private lateinit var tilPollInterval: TextInputLayout
    private lateinit var etApiBaseUrl: TextInputEditText
    private lateinit var etDeviceId: TextInputEditText
    private lateinit var etDeviceName: TextInputEditText
    private lateinit var etApiToken: TextInputEditText
    private lateinit var etPollInterval: TextInputEditText
    private lateinit var etBlacklist: TextInputEditText
    private lateinit var switchWindowTitle: MaterialSwitch
    private lateinit var switchPrivacyPause: MaterialSwitch

    private val handler = Handler(Looper.getMainLooper())
    private val refreshRunnable = object : Runnable {
        override fun run() {
            refreshStatus()
            handler.postDelayed(this, 3_000L)
        }
    }

    private val notificationPermissionLauncher =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) { /* 结果不影响主流程 */ }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        bindViews()
        ensureBridgeInitialized()
        loadFormFromBridge()
        setupActions()
        requestNotificationPermissionIfNeeded()
    }

    override fun onResume() {
        super.onResume()
        refreshStatus()
        handler.removeCallbacks(refreshRunnable)
        handler.postDelayed(refreshRunnable, 3_000L)
    }

    override fun onPause() {
        handler.removeCallbacks(refreshRunnable)
        super.onPause()
    }

    private fun bindViews() {
        tvServiceStatus = findViewById(R.id.tv_service_status)
        tvCurrentApp = findViewById(R.id.tv_current_app)
        tvLastReport = findViewById(R.id.tv_last_report)

        tilApiBaseUrl = findViewById(R.id.til_api_base_url)
        tilDeviceId = findViewById(R.id.til_device_id)
        tilPollInterval = findViewById(R.id.til_poll_interval_ms)
        etApiBaseUrl = findViewById(R.id.et_api_base_url)
        etDeviceId = findViewById(R.id.et_device_id)
        etDeviceName = findViewById(R.id.et_device_name)
        etApiToken = findViewById(R.id.et_api_token)
        etPollInterval = findViewById(R.id.et_poll_interval_ms)
        etBlacklist = findViewById(R.id.et_blacklist)
        switchWindowTitle = findViewById(R.id.switch_report_window_title)
        switchPrivacyPause = findViewById(R.id.switch_privacy_pause)
    }

    private fun configPath(): String =
        File(filesDir, "config.json").absolutePath

    private fun ensureBridgeInitialized() {
        try {
            NativeBridge.ensureInitialized(this)
        } catch (t: Throwable) {
            Toast.makeText(this, "JNI 初始化失败：${t.message}", Toast.LENGTH_LONG).show()
        }
    }

    private fun loadFormFromBridge() {
        val json = NativeBridge.getConfigJson() ?: return
        val form = try {
            ConfigMapper.toForm(json)
        } catch (_: Exception) {
            return
        }
        etApiBaseUrl.setText(form.apiBaseUrl)
        etDeviceId.setText(form.deviceId)
        etDeviceName.setText(form.deviceName)
        etApiToken.setText(form.apiToken)
        etPollInterval.setText(form.pollIntervalMs)
        etBlacklist.setText(form.blacklistCsv)
        switchWindowTitle.isChecked = form.reportWindowTitle
        switchPrivacyPause.isChecked = form.privacyPause
    }

    private fun setupActions() {
        findViewById<MaterialButton>(R.id.btn_save).setOnClickListener { saveForm() }

        // ① 无障碍服务：跳系统设置页 + 步骤文案
        findViewById<MaterialButton>(R.id.btn_enable_accessibility).setOnClickListener {
            openOrExplain(
                Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS),
                getString(R.string.accessibility_steps)
            )
        }

        // ② 电池优化白名单
        findViewById<MaterialButton>(R.id.btn_battery_optimization).setOnClickListener {
            val intent = Intent(
                Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS,
                Uri.parse("package:$packageName")
            )
            openOrExplain(intent, getString(R.string.battery_steps))
        }

        // ③ 国产 ROM 自启动：常见厂商设置页逐个尝试，失败给文案
        findViewById<MaterialButton>(R.id.btn_autostart).setOnClickListener { openAutostart() }
    }

    private fun saveForm() {
        val form = ConfigMapper.Form(
            apiBaseUrl = etApiBaseUrl.text?.toString().orEmpty(),
            deviceId = etDeviceId.text?.toString().orEmpty(),
            deviceName = etDeviceName.text?.toString().orEmpty(),
            apiToken = etApiToken.text?.toString().orEmpty(),
            pollIntervalMs = etPollInterval.text?.toString().orEmpty(),
            reportWindowTitle = switchWindowTitle.isChecked,
            blacklistCsv = etBlacklist.text?.toString().orEmpty(),
            privacyPause = switchPrivacyPause.isChecked
        )
        val errors = ConfigMapper.validate(form)
        tilApiBaseUrl.error = errors.apiBaseUrl
        tilDeviceId.error = errors.deviceId
        tilPollInterval.error = errors.pollIntervalMs
        if (!errors.isEmpty) return

        val existing = NativeBridge.getConfigJson() ?: "{}"
        val merged = try {
            ConfigMapper.mergeInto(existing, form)
        } catch (t: Throwable) {
            Toast.makeText(this, getString(R.string.saved_fail, t.message), Toast.LENGTH_LONG).show()
            return
        }
        val ok = try {
            NativeBridge.saveConfigJson(merged)
        } catch (t: Throwable) {
            false
        }
        if (ok) {
            Toast.makeText(this, R.string.saved_ok, Toast.LENGTH_SHORT).show()
            refreshStatus()
        } else {
            Toast.makeText(this, getString(R.string.saved_fail, "validate/save 失败"), Toast.LENGTH_LONG).show()
        }
    }

    private fun refreshStatus() {
        val json = try {
            NativeBridge.getStatusJson()
        } catch (_: Throwable) {
            null
        }
        if (json == null) {
            tvServiceStatus.setText(R.string.status_value_unknown)
            tvCurrentApp.setText(R.string.status_none)
            tvLastReport.setText(R.string.status_none)
            return
        }
        tvServiceStatus.text = if (isAccessibilityEnabled()) {
            getString(R.string.status_value_running)
        } else {
            getString(R.string.status_value_stopped)
        }
        tvCurrentApp.text = StatusFormatter.currentAppLabel(json)
        tvLastReport.text = StatusFormatter.lastReportLabel(json)
    }

    private fun isAccessibilityEnabled(): Boolean {
        val expected = ComponentName(this, ForegroundWatchService::class.java)
        val enabled = Settings.Secure.getString(
            contentResolver,
            Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES
        ) ?: return false
        return enabled.split(':').any { raw ->
            ComponentName.unflattenFromString(raw)?.equals(expected) == true
        }
    }

    private fun openOrExplain(intent: Intent, fallback: String) {
        try {
            startActivity(intent)
        } catch (_: Exception) {
            Toast.makeText(
                this,
                getString(R.string.permission_jump_failed, fallback),
                Toast.LENGTH_LONG
            ).show()
        }
    }

    private fun openAutostart() {
        val candidates = listOf(
            // 小米
            Intent().setComponent(ComponentName("com.miui.securitycenter", "com.miui.permcenter.autostart.AutoStartManagementActivity")),
            // 华为
            Intent().setComponent(ComponentName("com.huawei.systemmanager", "com.huawei.systemmanager.startupmgr.ui.StartupNormalAppListActivity")),
            // OPPO
            Intent().setComponent(ComponentName("com.coloros.safecenter", "com.coloros.safecenter.permission.startup.StartupAppListActivity")),
            // vivo
            Intent().setComponent(ComponentName("com.vivo.permissionmanager", "com.vivo.permissionmanager.activity.BgStartUpManagerActivity")),
            // 三星
            Intent().setComponent(ComponentName("com.samsung.android.lool", "com.samsung.android.sm.ui.battery.BatteryActivity")),
            // 通用应用详情页兜底
            Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:$packageName"))
        )
        for (intent in candidates) {
            try {
                startActivity(intent)
                return
            } catch (_: Exception) {
                // 尝试下一个
            }
        }
        Toast.makeText(this, getString(R.string.autostart_steps), Toast.LENGTH_LONG).show()
    }

    private fun requestNotificationPermissionIfNeeded() {
        if (Build.VERSION.SDK_INT >= 33 &&
            ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            notificationPermissionLauncher.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
    }

    companion object {
        /** 供服务/接收器判断无障碍是否启用（复用同一判定逻辑）。 */
        fun isAccessibilityEnabled(context: Context): Boolean {
            val expected = ComponentName(context, ForegroundWatchService::class.java)
            val enabled = Settings.Secure.getString(
                context.contentResolver,
                Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES
            ) ?: return false
            return enabled.split(':').any { raw ->
                ComponentName.unflattenFromString(raw)?.equals(expected) == true
            }
        }
    }
}
