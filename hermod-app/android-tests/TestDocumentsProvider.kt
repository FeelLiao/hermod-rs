package org.hermod.transfer

import android.database.Cursor
import android.database.MatrixCursor
import android.os.CancellationSignal
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract.Document
import android.provider.DocumentsContract.Root
import android.provider.DocumentsProvider
import java.io.FileNotFoundException
import kotlin.concurrent.thread

// 只编译到调试 APK；让仪器测试走真实 DocumentsUI 和授权 URI 流程。
class TestDocumentsProvider : DocumentsProvider() {
    override fun onCreate() = true
    override fun queryRoots(projection: Array<out String>?): Cursor {
        val cursor = MatrixCursor(projection ?: arrayOf(Root.COLUMN_ROOT_ID, Root.COLUMN_DOCUMENT_ID, Root.COLUMN_TITLE, Root.COLUMN_FLAGS, Root.COLUMN_MIME_TYPES))
        cursor.newRow().add(Root.COLUMN_ROOT_ID, "root").add(Root.COLUMN_DOCUMENT_ID, "root")
            .add(Root.COLUMN_TITLE, "Hermod test documents").add(Root.COLUMN_FLAGS, Root.FLAG_LOCAL_ONLY).add(Root.COLUMN_MIME_TYPES, "text/plain")
        return cursor
    }
    private fun documents(projection: Array<out String>?, id: String): Cursor {
        if (id !in listOf("root", "fixture", "revoked")) throw FileNotFoundException("Unknown document")
        val cursor = MatrixCursor(projection ?: arrayOf(Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME, Document.COLUMN_MIME_TYPE, Document.COLUMN_FLAGS, Document.COLUMN_SIZE))
        cursor.newRow().add(Document.COLUMN_DOCUMENT_ID, id).add(Document.COLUMN_DISPLAY_NAME, if (id == "root") "Hermod test documents" else if (id == "fixture") "fixture.txt" else "revoked.txt")
            .add(Document.COLUMN_MIME_TYPE, if (id == "root") Document.MIME_TYPE_DIR else "text/plain").add(Document.COLUMN_FLAGS, 0).add(Document.COLUMN_SIZE, null)
        return cursor
    }
    override fun queryDocument(documentId: String, projection: Array<out String>?) = documents(projection, documentId)
    override fun queryChildDocuments(parentDocumentId: String, projection: Array<out String>?, sortOrder: String?): Cursor =
        android.database.MergeCursor(arrayOf(documents(projection, "fixture"), documents(projection, "revoked")))
    override fun openDocument(documentId: String, mode: String, signal: CancellationSignal?): ParcelFileDescriptor {
        if (documentId == "revoked") throw SecurityException("Document access revoked by provider")
        if (documentId != "fixture" || mode != "r") throw FileNotFoundException("Read-only fixture")
        val pipe = ParcelFileDescriptor.createPipe()
        thread(isDaemon = true) { runCatching { ParcelFileDescriptor.AutoCloseOutputStream(pipe[1]).use { it.write("Hermod SAF fixture".toByteArray()) } } }
        return pipe[0]
    }
}
