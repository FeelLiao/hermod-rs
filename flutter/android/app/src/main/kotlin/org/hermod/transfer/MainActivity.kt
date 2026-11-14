package org.hermod.transfer

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.StatFs
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import androidx.activity.result.contract.ActivityResultContracts
import io.flutter.embedding.android.FlutterFragmentActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import java.io.File
import java.io.IOException
import java.util.UUID
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/** SAF URIs stay native. Rust receives only complete app-private staged files. */
class MainActivity : FlutterFragmentActivity(), MethodChannel.MethodCallHandler {
    private val app get() = application as HermodApplication
    private var operationId: String? = null
    private var operationKind: String? = null
    private var pickerOutstanding = false
    private var pending: MethodChannel.Result? = null
    private var exportSource: File? = null
    private var exportCopyRunning = false
    private var cancelled = AtomicBoolean(false)
    private val io = Executors.newSingleThreadExecutor()

    private val openDocument = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { response ->
        pickerOutstanding = false
        val result = pending ?: return@registerForActivityResult
        val uri = response.data?.data
        if (response.resultCode != RESULT_OK || uri == null) {
            finishResult(result, null)
        } else {
            runIo(result) { stageDocument(uri) }
        }
    }
    private val createDocument = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { response ->
        pickerOutstanding = false
        val uri = response.data?.data
        val result = pending
        if (result == null) {
            if (uri != null && response.resultCode == RESULT_OK) deleteExport(uri)
            return@registerForActivityResult
        }
        val source = exportSource
        if (response.resultCode != RESULT_OK || uri == null || source == null || cancelled.get()) {
            if (uri != null && response.resultCode == RESULT_OK) {
                deleteExport(uri)
            }
            finishResult(result, false)
        } else {
            exportCopyRunning = true
            runIo(result, { deleteExport(uri) }) {
                try {
                    contentResolver.openOutputStream(uri, "wt")?.use { output ->
                        source.inputStream().use { input -> copyChunks(input, output, false) }
                    } ?: throw IOException("Document provider did not open an output stream")
                    true
                } catch (error: Exception) {
                    // ACTION_CREATE_DOCUMENT creates a new document, never an existing inbox file.
                    try { DocumentsContract.deleteDocument(contentResolver, uri) } catch (cleanup: Exception) {
                        android.util.Log.w("HermodSAF", "Unable to remove partial export", cleanup)
                    }
                    throw error
                } finally {
                    app.exportSources.remove(source.path)
                }
            }
        }
    }
    private val notificationPermission = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        pickerOutstanding = false
        val result = pending ?: return@registerForActivityResult
        if (granted) startServiceFor(result) else finishResult(result, false)
    }

    override fun provideFlutterEngine(context: android.content.Context): FlutterEngine =
        (application as HermodApplication).engine

    override fun shouldDestroyEngineWithHost(): Boolean = false

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        // Application-created engine already auto-registers plugins exactly once.
        app.attach(this)
    }

    override fun onMethodCall(call: MethodCall, result: MethodChannel.Result) {
        when (call.method) {
            "cancelPlatformOperation" -> {
                val id = call.argument<String>("operationId")
                val matches = id != null && id == operationId && pending != null
                if (matches) {
                    cancelled.set(true)
                    if (operationKind == "startTransferService") app.stopTransfer(id, "cancelled")
                }
                result.success(matches)
                return
            }
            "pickSource", "exportFile", "startTransferService" -> Unit
            else -> { result.notImplemented(); return }
        }
        if (pending != null || pickerOutstanding) { result.error("busy", "Another platform operation is active", null); return }
        val id = call.argument<String>("operationId")
        if (id.isNullOrBlank() || id.length > 128) {
            result.error("invalid_arguments", "Unique operationId (1..128 characters) is required", null)
            return
        }
        operationId = id
        operationKind = call.method
        pending = result
        cancelled = AtomicBoolean(false)
        try {
            when (call.method) {
                "pickSource" -> { pickerOutstanding = true; openDocument.launch(Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                    type = "*/*"
                    addCategory(Intent.CATEGORY_OPENABLE)
                    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                }) }
                "exportFile" -> {
                    val path = call.argument<String>("sourcePath")
                        ?: throw IllegalArgumentException("sourcePath is required")
                    val source = File(path)
                    require(source.isAbsolute && source.isFile) { "sourcePath must be a readable local file" }
                    // Only export app-private files; no URI strings or arbitrary external paths.
                    val canonical = source.canonicalFile
                    val roots = listOf(filesDir.canonicalFile, cacheDir.canonicalFile)
                    require(roots.any { canonical.path.startsWith(it.path + File.separator) }) {
                        "sourcePath must be app-private"
                    }
                    exportSource = canonical
                    exportCopyRunning = false
                    app.exportSources.add(canonical.path)
                    pickerOutstanding = true
                    createDocument.launch(Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
                        type = "application/octet-stream"
                        addCategory(Intent.CATEGORY_OPENABLE)
                        putExtra(Intent.EXTRA_TITLE, safeName(call.argument<String>("name") ?: source.name))
                    })
                }
                "startTransferService" -> {
                    // Do not attempt a prohibited background FGS start. Caller must start before transfer.
                    if (!lifecycle.currentState.isAtLeast(androidx.lifecycle.Lifecycle.State.RESUMED)) {
                        finishResult(result, false)
                    } else if (checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
                        pickerOutstanding = true
                        notificationPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
                    } else startServiceFor(result)
                }
            }
        } catch (error: Exception) {
            pickerOutstanding = false
            failResult(result, "platform_error", error)
        }
    }

    private fun startServiceFor(result: MethodChannel.Result) {
        try {
            if (cancelled.get() || !lifecycle.currentState.isAtLeast(androidx.lifecycle.Lifecycle.State.RESUMED)) {
                finishResult(result, false)
                return
            }
            val id = operationId ?: throw IllegalStateException("Missing operation ID")
            app.startTransfer(id) { ready ->
                if (pending === result) finishResult(result, ready && !cancelled.get())
            }
        } catch (error: Exception) { failResult(result, "service_start_failed", error) }
    }

    private fun stageDocument(uri: Uri): Map<String, String> {
        var staged: File? = null
        try {
            // The transient picker read grant suffices for this immediate copy. Never take/release
            // a persisted grant that might already belong to another feature of the application.
            var name = "source.bin"
            var size: Long? = null
            contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { cursor ->
                if (cursor.moveToFirst()) {
                    val nameIndex = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                    if (nameIndex >= 0 && !cursor.isNull(nameIndex)) name = safeName(cursor.getString(nameIndex))
                    val sizeIndex = cursor.getColumnIndex(OpenableColumns.SIZE)
                    if (sizeIndex >= 0 && !cursor.isNull(sizeIndex)) size = cursor.getLong(sizeIndex).takeIf { it >= 0 }
                }
            }
            val directory = File(filesDir, "source-staging")
            check(directory.isDirectory || directory.mkdirs()) { "Cannot create private staging directory" }
            val available = StatFs(directory.path).availableBytes - SPACE_RESERVE
            if (size != null && size!! > available) throw IOException("Insufficient private staging space")
            // Exclusive creation: never truncate a colliding path, even if the provider supplied its name.
            val target = File(directory, "${UUID.randomUUID()}-$name")
            if (!target.createNewFile()) throw IOException("Staging name collision")
            staged = target
            contentResolver.openInputStream(uri)?.use { input ->
                target.outputStream().use { output -> copyChunks(input, output, true) }
            } ?: throw IOException("Document provider did not open an input stream")
            if (cancelled.get()) throw IOException("Operation cancelled")
            return mapOf("path" to target.canonicalPath, "name" to name)
        } catch (error: Exception) {
            staged?.let { if (it.exists() && !it.delete()) android.util.Log.w("HermodSAF", "Partial staging cleanup failed") }
            throw error
        }
    }

    private fun copyChunks(input: java.io.InputStream, output: java.io.OutputStream, budget: Boolean) {
        val buffer = ByteArray(64 * 1024)
        while (true) {
            if (cancelled.get() || Thread.currentThread().isInterrupted) throw IOException("Operation cancelled")
            val count = input.read(buffer)
            if (count < 0) break
            if (budget && StatFs(filesDir.path).availableBytes < SPACE_RESERVE + count) {
                throw IOException("Insufficient private staging space")
            }
            output.write(buffer, 0, count)
        }
        output.flush()
    }

    private fun deleteExport(uri: Uri) {
        app.cleanupIo.execute {
            try { DocumentsContract.deleteDocument(contentResolver, uri) }
            catch (error: Exception) { android.util.Log.w("HermodSAF", "Export cleanup failed", error) }
        }
    }

    private fun runIo(result: MethodChannel.Result, discard: (() -> Unit)? = null, action: () -> Any) {
        io.execute {
            try {
                val value = action()
                runOnUiThread {
                    if (pending === result && !isDestroyed && !cancelled.get()) finishResult(result, value)
                    else {
                        discard?.invoke()
                        if (value is Map<*, *>) (value["path"] as? String)?.let { File(it).delete() }
                        if (pending === result) finishResult(result, if (operationKind == "exportFile") false else null)
                    }
                }
            } catch (error: Exception) {
                runOnUiThread {
                    if (pending === result && !isDestroyed) {
                        if (cancelled.get()) finishResult(result, if (exportSource != null) false else null)
                        else failResult(result, "file_io", error)
                    }
                }
            }
        }
    }

    private fun finishResult(result: MethodChannel.Result, value: Any?) {
        if (pending === result) {
            pending = null
            if (!exportCopyRunning) exportSource?.let { app.exportSources.remove(it.path) }
            exportSource = null
            result.success(value)
        }
    }

    private fun failResult(result: MethodChannel.Result, code: String, error: Exception) {
        if (pending === result) {
            pending = null
            if (!exportCopyRunning) exportSource?.let { app.exportSources.remove(it.path) }
            exportSource = null
            result.error(code, error.message ?: "Platform operation failed", null)
        }
    }

    override fun onCreate(savedInstanceState: android.os.Bundle?) {
        pickerOutstanding = savedInstanceState?.getBoolean("hermod.pickerOutstanding") ?: false
        super.onCreate(savedInstanceState)
    }

    override fun onSaveInstanceState(outState: android.os.Bundle) {
        outState.putBoolean("hermod.pickerOutstanding", pickerOutstanding)
        super.onSaveInstanceState(outState)
    }

    override fun cleanUpFlutterEngine(flutterEngine: FlutterEngine) {
        app.detach(this)
        super.cleanUpFlutterEngine(flutterEngine)
    }

    override fun onDestroy() {
        cancelled.set(true)
        if (operationKind == "startTransferService" && pending != null) app.stopTransfer(operationId, "cancelled")
        pending?.let { failResult(it, "activity_destroyed", IOException("Activity was destroyed; retry the operation")) }
        app.detach(this)
        // Drain queued cancellation work so its finally releases process-owned export pins.
        io.shutdown()
        super.onDestroy()
    }

    companion object {
        private const val SPACE_RESERVE = 64L * 1024 * 1024
        internal fun safeName(raw: String): String {
            val clean = raw.replace(Regex("[\\p{Cntrl}/\\\\:*?\"<>|]"), "_").trim().trim('.')
                .take(48).trim().trim('.')
            return clean.ifBlank { "source.bin" }
        }
    }
}
