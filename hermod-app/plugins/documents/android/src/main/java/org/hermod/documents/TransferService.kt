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
        @Volatile private var phaseText = "Connecting"
        @Volatile private var transferredText = "0 KiB transferred"
        @Volatile private var cancelText = "Cancel"
        @Volatile private var channelText = "File transfers"
        fun localNetworkAllowed(context: Context) = Build.VERSION.SDK_INT < 37 ||
            context.checkSelfPermission("android.permission.ACCESS_LOCAL_NETWORK") == PackageManager.PERMISSION_GRANTED
        fun update(phase: String, bytes: Long, total: Long?, transferred: String, cancel: String, channel: String) {
            phaseText = phase
            transferredText = transferred
            cancelText = cancel
            channelText = channel
            instance.get()?.let { service ->
                service.createChannel()
                service.getSystemService(NotificationManager::class.java).notify(1001, service.notification(phase, bytes, total))
            }
        }
    }
    private var wakeLock: PowerManager.WakeLock? = null
    private fun notification(phase: String, bytes: Long, total: Long?): Notification {
        val launch = packageManager.getLaunchIntentForPackage(packageName)
        val cancel = PendingIntent.getService(this, 1, Intent(this, TransferService::class.java).setAction("cancel"), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        return Notification.Builder(this, "transfer").setContentTitle("Hermod · $phase")
            .setContentText(transferredText).setSmallIcon(android.R.drawable.stat_sys_upload)
            .setOngoing(true).setOnlyAlertOnce(true)
            .setProgress(100, if (total != null && total > 0) ((bytes.toDouble() / total) * 100).toInt().coerceIn(0, 100) else 0, total == null)
            .addAction(Notification.Action.Builder(null, cancelText, cancel).build())
            .apply { if (launch != null) setContentIntent(PendingIntent.getActivity(this@TransferService, 0, launch, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)) }.build()
    }
    private fun createChannel() {
        getSystemService(NotificationManager::class.java).createNotificationChannel(NotificationChannel("transfer", channelText, NotificationManager.IMPORTANCE_LOW))
    }
    override fun onCreate() {
        super.onCreate()
        createChannel()
        instance = WeakReference(this)
        startForeground(1001, notification(phaseText, 0, null))
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
