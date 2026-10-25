package org.hermod.documents

import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.OpenableColumns
import androidx.activity.result.ActivityResult
import androidx.core.content.ContextCompat
import androidx.documentfile.provider.DocumentFile
import app.tauri.PermissionState
import app.tauri.annotation.*
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class UriArgs { lateinit var uri: String }
@InvokeArg
class SaveArgs { lateinit var name: String; var folder: String? = null }
@InvokeArg
class ActiveArgs { var active: Boolean = false }
@InvokeArg
class ProgressArgs { var phase: String = "Connecting"; var bytes: Long = 0; var total: Long? = null }

@TauriPlugin(permissions = [
    Permission(alias = "lan", strings = ["android.permission.ACCESS_LOCAL_NETWORK"]),
    Permission(alias = "notifications", strings = ["android.permission.POST_NOTIFICATIONS"])
])
class DocumentsPlugin(private val activity: Activity) : Plugin(activity) {
    private fun result(invoke: Invoke, action: () -> JSObject) {
        try { invoke.resolve(action()) } catch (error: Exception) {
            invoke.reject(error.message ?: "Android document operation failed; verified inbox copy retained")
        }
    }
    @Command fun palette(invoke: Invoke) = result(invoke) {
        JSObject().put("seed", String.format("#%06X", activity.getColor(android.R.color.system_accent1_500) and 0xffffff))
    }
    @Command fun prepare(invoke: Invoke) {
        if (Build.VERSION.SDK_INT >= 37 && !TransferService.localNetworkAllowed(activity)) {
            requestPermissionForAlias("lan", invoke, "afterLan")
        } else afterLan(invoke)
    }
    @PermissionCallback fun afterLan(invoke: Invoke) {
        if (!TransferService.localNetworkAllowed(activity)) {
            invoke.reject("Local network permission denied. Peer discovery and direct transfers need this permission. Enable Local network in Android app settings.")
            return
        }
        if (getPermissionState("notifications") == PermissionState.PROMPT) {
            requestPermissionForAlias("notifications", invoke, "afterNotifications")
        } else invoke.resolve()
    }
    @PermissionCallback fun afterNotifications(invoke: Invoke) { invoke.resolve() }
    @Command fun pickSource(invoke: Invoke) {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).setType("*/*").addCategory(Intent.CATEGORY_OPENABLE)
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        startActivityForResult(invoke, intent, "sourcePicked")
    }
    @ActivityCallback fun sourcePicked(invoke: Invoke, response: ActivityResult) = result(invoke) {
        val uri = response.data?.data.takeIf { response.resultCode == Activity.RESULT_OK }
        val output = JSObject()
        if (uri != null) {
            require(uri.scheme == "content") { "Expected an authorized content URI" }
            output.put("uri", uri.toString()).put("name", "file")
            activity.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { cursor ->
                if (cursor.moveToFirst()) {
                    val name = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                    val size = cursor.getColumnIndex(OpenableColumns.SIZE)
                    if (name >= 0 && !cursor.isNull(name)) output.put("name", cursor.getString(name))
                    if (size >= 0 && !cursor.isNull(size) && cursor.getLong(size) >= 0) output.put("size", cursor.getLong(size))
                }
            }
        }
        output
    }
    @Command fun pickFolder(invoke: Invoke) {
        startActivityForResult(invoke, Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).addFlags(
            Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION
        ), "folderPicked")
    }
    @ActivityCallback fun folderPicked(invoke: Invoke, response: ActivityResult) = result(invoke) {
        val data = response.data
        val uri = data?.data.takeIf { response.resultCode == Activity.RESULT_OK }
        val output = JSObject()
        if (uri != null) {
            activity.contentResolver.takePersistableUriPermission(uri, data!!.flags and (Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION))
            // 只有一个默认目录；及时释放旧授权，防止累积到系统上限。
            activity.contentResolver.persistedUriPermissions.filter { it.uri != uri }.forEach {
                runCatching { activity.contentResolver.releasePersistableUriPermission(it.uri,
                    (if (it.isReadPermission) Intent.FLAG_GRANT_READ_URI_PERMISSION else 0) or
                    (if (it.isWritePermission) Intent.FLAG_GRANT_WRITE_URI_PERMISSION else 0)) }
            }
            output.put("uri", uri.toString())
        }
        output
    }
    @Command fun createExport(invoke: Invoke) {
        val args = invoke.parseArgs(SaveArgs::class.java)
        val folder = args.folder
        if (folder != null) result(invoke) {
            val tree = DocumentFile.fromTreeUri(activity, Uri.parse(folder)) ?: error("Export folder is no longer authorized; choose it again")
            check(tree.canWrite()) { "Export folder permission was revoked; choose it again" }
            val document = tree.createFile("application/octet-stream", args.name) ?: error("Cannot create export; inbox copy retained")
            JSObject().put("uri", document.uri.toString())
        } else {
            startActivityForResult(invoke, Intent(Intent.ACTION_CREATE_DOCUMENT).setType("application/octet-stream")
                .addCategory(Intent.CATEGORY_OPENABLE).putExtra(Intent.EXTRA_TITLE, args.name), "exportPicked")
        }
    }
    @ActivityCallback fun exportPicked(invoke: Invoke, response: ActivityResult) = result(invoke) {
        JSObject().apply { if (response.resultCode == Activity.RESULT_OK) response.data?.data?.let { put("uri", it.toString()) } }
    }
    @Command fun removePartial(invoke: Invoke) = result(invoke) {
        val uri = Uri.parse(invoke.parseArgs(UriArgs::class.java).uri)
        android.provider.DocumentsContract.deleteDocument(activity.contentResolver, uri)
        JSObject()
    }
    @Command fun setActive(invoke: Invoke) = result(invoke) {
        val intent = Intent(activity, TransferService::class.java)
        if (invoke.parseArgs(ActiveArgs::class.java).active) {
            check(TransferService.localNetworkAllowed(activity)) { "Local network permission denied; transfer cannot start" }
            TransferService.cancelRequested = false
            ContextCompat.startForegroundService(activity, intent)
        } else activity.stopService(intent)
        JSObject()
    }
    @Command fun progress(invoke: Invoke) = result(invoke) {
        val args = invoke.parseArgs(ProgressArgs::class.java)
        TransferService.update(args.phase, args.bytes, args.total)
        JSObject().put("cancel", TransferService.cancelRequested)
    }
}
