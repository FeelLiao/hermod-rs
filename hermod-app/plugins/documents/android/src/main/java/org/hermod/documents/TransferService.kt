package org.hermod.documents

import android.app.*
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.*
import java.lang.ref.WeakReference

// 服务维持进程优先级；Rust 持有任务，网页休眠和旋转不影响传输。
class TransferService : Service() {
    companion object {
        @Volatile var cancelRequested = false
        private var instance = WeakReference<TransferService>(null)
        fun localNetworkAllowed(context: Context) = Build.VERSION.SDK_INT < 37 ||
            context.checkSelfPermission("android.permission.ACCESS_LOCAL_NETWORK") == PackageManager.PERMISSION_GRANTED
        fun update(phase: String, bytes: Long, total: Long?) {
            instance.get()?.let { service -> service.getSystemService(NotificationManager::class.java)
                .notify(1001, service.notification(phase, bytes, total)) }
        }
    }
    private var wakeLock: PowerManager.WakeLock? = null
    private fun notification(phase: String, bytes: Long, total: Long?): Notification {
        val launch = packageManager.getLaunchIntentForPackage(packageName)
        val cancel = PendingIntent.getService(this, 1, Intent(this, TransferService::class.java).setAction("cancel"), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        return Notification.Builder(this, "transfer").setContentTitle("Hermod · $phase")
            .setContentText("${bytes / 1024} KiB transferred").setSmallIcon(android.R.drawable.stat_sys_upload)
            .setOngoing(true).setOnlyAlertOnce(true)
            .setProgress(100, if (total != null && total > 0) ((bytes.toDouble() / total) * 100).toInt().coerceIn(0, 100) else 0, total == null)
            .addAction(Notification.Action.Builder(null, "Cancel", cancel).build())
            .apply { if (launch != null) setContentIntent(PendingIntent.getActivity(this@TransferService, 0, launch, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)) }.build()
    }
    override fun onCreate() {
        super.onCreate()
        getSystemService(NotificationManager::class.java).createNotificationChannel(NotificationChannel("transfer", "File transfers", NotificationManager.IMPORTANCE_LOW))
        instance = WeakReference(this)
        startForeground(1001, notification("Connecting", 0, null))
        wakeLock = getSystemService(PowerManager::class.java).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "Hermod:transfer").apply { acquire(6 * 60 * 60 * 1000L) }
    }
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == "cancel") cancelRequested = true
        return START_NOT_STICKY
    }
    override fun onBind(intent: Intent?): IBinder? = null
    @androidx.annotation.RequiresApi(35)
    override fun onTimeout(startId: Int, fgsType: Int) { cancelRequested = true; stopSelf() }
    override fun onDestroy() {
        cancelRequested = true
        instance.clear()
        wakeLock?.let { if (it.isHeld) it.release() }
        stopForeground(STOP_FOREGROUND_REMOVE)
        super.onDestroy()
    }
}
