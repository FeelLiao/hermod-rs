package org.hermod.transfer

import android.app.Application
import android.content.Intent
import android.os.Handler
import android.os.Looper
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

/** One engine/channel per process. All coordinator transitions run on the main thread. */
class HermodApplication : Application() {
    val cleanupIo = java.util.concurrent.Executors.newSingleThreadExecutor()
    val exportSources = java.util.concurrent.ConcurrentHashMap.newKeySet<String>()
    private var activity: MainActivity? = null
    private val main = Handler(Looper.getMainLooper())
    private var transferId: String? = null
    private var ready: ((Boolean) -> Unit)? = null
    private var stopping = false
    private var service: TransferService? = null
    private val stopWaiters = mutableListOf<MethodChannel.Result>()
    private val readyDeadline = Runnable {
        ready?.let { callback ->
            ready = null
            callback(false)
            stopTransfer(transferId, "start_timeout")
        }
    }
    val engine: FlutterEngine by lazy {
        // Constructor registers generated plugins once; the host must not register them again.
        FlutterEngine(this)
    }
    private val channel: MethodChannel by lazy {
        MethodChannel(engine.dartExecutor.binaryMessenger, "hermod/platform").also { channel ->
            channel.setMethodCallHandler { call, result ->
                // Process-owned cleanup must remain callable after the UI is destroyed.
                when (call.method) {
                    "stopTransferService" -> {
                        val reason = call.argument<String>("reason") ?: "completed"
                        if (reason !in listOf("completed", "cancelled", "failed")) {
                            result.error("invalid_arguments", "Invalid stop reason", null)
                        } else stopTransfer(call.argument<String>("operationId"), reason, result)
                    }
                    "releaseStagedSource" -> releaseStagedSource(call.argument<String>("path"), result)
                    "getSupportDirectory" -> {
                        try {
                            val root = java.io.File(filesDir, "hermod")
                            check(root.isDirectory || root.mkdirs()) { "Cannot create application support directory" }
                            result.success(root.canonicalPath)
                        } catch (error: Exception) { result.error("storage_error", error.message, null) }
                    }
                    else -> {
                        val host = activity
                        if (host == null) result.error("activity_unavailable", "Open the application and retry", null)
                        else host.onMethodCall(call, result)
                    }
                }
            }
        }
    }

    private fun releaseStagedSource(path: String?, result: MethodChannel.Result) {
        try {
            require(path != null) { "path is required" }
            val file = java.io.File(path)
            val directory = java.io.File(filesDir, "source-staging")
            val root = directory.canonicalFile
            val canonical = file.canonicalFile
            require(!java.nio.file.Files.isSymbolicLink(directory.toPath()) &&
                canonical.path !in exportSources && file.isAbsolute && file.absolutePath == canonical.path &&
                canonical.parentFile == root &&
                Regex("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}-.*").matches(file.name) &&
                !java.nio.file.Files.isSymbolicLink(file.toPath())) { "Not an owned staging file" }
            result.success(canonical.isFile && canonical.delete())
        } catch (error: Exception) { result.error("storage_error", error.message, null) }
    }

    fun attach(host: MainActivity) { channel; activity = host }
    fun detach(host: MainActivity) { if (activity === host) activity = null }

    fun startTransfer(id: String, callback: (Boolean) -> Unit) {
        check(transferId == null) { "A transfer service is active or stopping" }
        transferId = id
        ready = callback
        stopping = false
        main.postDelayed(readyDeadline, 10_000)
        try {
            startForegroundService(Intent(this, TransferService::class.java).putExtra("operationId", id))
        } catch (error: Exception) {
            main.removeCallbacks(readyDeadline)
            transferId = null
            ready = null
            throw error
        }
    }

    fun serviceReady(host: TransferService, id: String): Boolean {
        if (transferId != id || stopping) return false
        service = host
        main.removeCallbacks(readyDeadline)
        val callback = ready
        ready = null
        callback?.invoke(true)
        return true
    }

    fun stopTransfer(id: String?, reason: String, result: MethodChannel.Result? = null): Boolean {
        if (id == null || transferId != id) { result?.success(false); return false }
        result?.let { stopWaiters.add(it) }
        if (!stopping) {
            stopping = true
            main.removeCallbacks(readyDeadline)
            val callback = ready
            ready = null
            callback?.invoke(false)
            val running = service
            if (running != null) running.finish(reason)
            // A queued start still must enter foreground promptly. serviceReady rejects it,
            // then onStartCommand stops it; do not free the slot before onDestroy.
        }
        return true
    }

    fun serviceStopped(host: TransferService, id: String?, reason: String) {
        if (id == null || transferId != id || (service != null && service !== host)) return
        main.removeCallbacks(readyDeadline)
        val callback = ready
        ready = null
        transferId = null
        service = null
        stopping = false
        callback?.invoke(false)
        channel.invokeMethod("transferServiceStopped", mapOf("operationId" to id, "reason" to reason))
        val waiters = stopWaiters.toList()
        stopWaiters.clear()
        waiters.forEach { it.success(true) }
    }
}
