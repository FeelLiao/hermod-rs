package org.hermod.documents

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import androidx.core.content.ContextCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class DescribeArgs { lateinit var uri: String }
@InvokeArg
class ActiveArgs { var active: Boolean = false }

@TauriPlugin
class DocumentsPlugin(private val activity: Activity) : Plugin(activity) {
    @Command
    fun describe(invoke: Invoke) {
        try {
            val uri = Uri.parse(invoke.parseArgs(DescribeArgs::class.java).uri)
            require(uri.scheme == "content") { "Expected an authorized content URI" }
            val result = JSObject()
            activity.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { cursor ->
                if (cursor.moveToFirst()) {
                    val name = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                    val size = cursor.getColumnIndex(OpenableColumns.SIZE)
                    result.put("name", if (name >= 0) cursor.getString(name) else "file")
                    if (size >= 0 && !cursor.isNull(size) && cursor.getLong(size) >= 0) result.put("size", cursor.getLong(size))
                }
            }
            if (!result.has("name")) result.put("name", "file")
            invoke.resolve(result)
        } catch (error: Exception) { invoke.reject(error.message ?: "Cannot read document metadata") }
    }

    @Command
    fun setActive(invoke: Invoke) {
        try {
            val intent = Intent(activity, TransferService::class.java)
            if (invoke.parseArgs(ActiveArgs::class.java).active) {
                ContextCompat.startForegroundService(activity, intent)
            } else { activity.stopService(intent) }
            invoke.resolve()
        } catch (error: Exception) { invoke.reject(error.message ?: "Cannot start transfer service") }
    }
}
