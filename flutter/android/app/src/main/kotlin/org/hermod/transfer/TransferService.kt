package org.hermod.transfer

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import android.os.PowerManager

/** Keeps the process eligible for an already user-started transfer, not a resumable job. */
class TransferService : Service() {
    private var stopReason = "stopped"
    private var operationId: String? = null
    private var failure: Exception? = null
    private val deadline = android.os.Handler(android.os.Looper.getMainLooper())
    private val expire = Runnable { finish("timeout") }
    private var wakeLock: PowerManager.WakeLock? = null

    override fun onCreate() {
        super.onCreate()
        try {
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel(
            CHANNEL, getString(R.string.transfer_channel), NotificationManager.IMPORTANCE_LOW
        ))
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val notification = Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_transfer)
            .setContentTitle(getString(R.string.transfer_notification_title))
            .setContentText(getString(R.string.transfer_notification_text))
            .setContentIntent(open)
            .setOngoing(true)
            .setCategory(Notification.CATEGORY_PROGRESS)
            .build()
        startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        // Foreground status alone does not keep CPU awake during a locked-screen transfer.
        wakeLock = getSystemService(PowerManager::class.java)
            .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "Hermod:transfer").apply {
                setReferenceCounted(false)
                acquire(6L * 60 * 60 * 1000)
            }
        deadline.postDelayed(expire, 6L * 60 * 60 * 1000)
        } catch (error: Exception) {
            failure = error
            android.util.Log.e("HermodFGS", "Foreground promotion failed", error)
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        operationId = intent?.getStringExtra("operationId")
        val id = operationId
        if (failure != null) finish("start_failed")
        else if (id == null || !(application as HermodApplication).serviceReady(this, id)) finish("cancelled")
        return START_NOT_STICKY
    }

    internal fun finish(reason: String) {
        stopReason = reason
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }
    override fun onBind(intent: Intent?): IBinder? = null

    // Android 15+ dataSync time budget expiry. Stop promptly; Dart must cancel its active task.
    override fun onTimeout(startId: Int, fgsType: Int) {
        finish("timeout")
    }

    override fun onDestroy() {
        wakeLock?.let { if (it.isHeld) it.release() }
        wakeLock = null
        deadline.removeCallbacks(expire)
        (application as HermodApplication).serviceStopped(this, operationId, stopReason)
        stopForeground(STOP_FOREGROUND_REMOVE)
        super.onDestroy()
    }

    companion object {
        private const val CHANNEL = "hermod_transfers"
        private const val NOTIFICATION_ID = 1001
    }
}
