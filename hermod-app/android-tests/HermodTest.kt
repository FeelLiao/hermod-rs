package org.hermod.transfer

import android.content.Intent
import android.os.Build
import android.view.View
import android.view.ViewGroup
import android.webkit.WebView
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.hermod.documents.TransferService
import org.json.JSONObject
import org.json.JSONTokener
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

@RunWith(AndroidJUnit4::class)
class HermodTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private val device = UiDevice.getInstance(instrumentation)
    private fun web(view: View): WebView? {
        if (view is WebView) return view
        if (view is ViewGroup) for (i in 0 until view.childCount) web(view.getChildAt(i))?.let { return it }
        return null
    }
    private fun js(scenario: ActivityScenario<MainActivity>, code: String): Any? {
        val latch = CountDownLatch(1)
        var value: Any? = null
        scenario.onActivity { activity ->
            val browser = web(activity.window.decorView) ?: error("WebView missing")
            browser.evaluateJavascript(code) { value = JSONTokener(it).nextValue(); latch.countDown() }
        }
        check(latch.await(10, TimeUnit.SECONDS)) { "JavaScript timed out" }
        return value
    }
    private fun waitFor(scenario: ActivityScenario<MainActivity>, expression: String) {
        repeat(100) { if (js(scenario, "Boolean($expression)") == true) return; Thread.sleep(100) }
        error("Web UI did not become ready: $expression")
    }
    private fun invoke(scenario: ActivityScenario<MainActivity>, command: String, args: String = "{}") {
        js(scenario, "window.hermodTestResult=null; window.__TAURI_INTERNALS__.invoke('$command',$args).then(value=>window.hermodTestResult={ok:true,value},error=>window.hermodTestResult={ok:false,error:String(error)}); void 0")
    }
    private fun result(scenario: ActivityScenario<MainActivity>): JSONObject {
        waitFor(scenario, "window.hermodTestResult")
        return JSONObject(js(scenario, "JSON.stringify(window.hermodTestResult)") as String)
    }
    private fun ready(scenario: ActivityScenario<MainActivity>) {
        waitFor(scenario, "document.querySelector('.material md-filled-button')")
    }
    @Test fun webUiRustSettingsAndActivityRecreation() {
        ActivityScenario.launch(MainActivity::class.java).let { scenario ->
            ready(scenario)
            invoke(scenario, "bootstrap")
            val info = result(scenario).getJSONObject("value")
            assertEquals("android", info.getString("platform"))
            assertEquals("wireguard", info.getJSONObject("settings").getJSONObject("connection").getString("transport"))
            assertTrue(info.getString("color_seed").startsWith("#"))
            val settings = info.getJSONObject("settings").put("words", 5)
            invoke(scenario, "save_settings", JSONObject().put("settings", settings).toString())
            assertTrue(result(scenario).getBoolean("ok"))
            scenario.recreate()
            ready(scenario)
            invoke(scenario, "bootstrap")
            assertEquals(5, result(scenario).getJSONObject("value").getJSONObject("settings").getInt("words"))
            settings.put("words", 3)
            invoke(scenario, "save_settings", JSONObject().put("settings", settings).toString())
            assertTrue(result(scenario).getBoolean("ok"))
        }
    }
    @Test fun safSelectionKeepsUnknownLengthAndReadsAuthorizedUri() {
        ActivityScenario.launch(MainActivity::class.java).let { scenario ->
            ready(scenario)
            invoke(scenario, "choose_source")
            device.wait(Until.hasObject(By.pkg("com.google.android.documentsui")), 10000)
            val root = device.findObject(By.text("Hermod test documents"))
            if (root == null) device.findObject(By.desc("Show roots"))?.click()
            check(device.wait(Until.findObject(By.text("Hermod test documents")), 10000) != null)
            device.findObject(By.text("Hermod test documents")).click()
            check(device.wait(Until.findObject(By.text("fixture.txt")), 10000) != null)
            device.findObject(By.text("fixture.txt")).click()
            val value = result(scenario).getJSONObject("value")
            assertEquals("fixture.txt", value.getString("name"))
            assertTrue(value.isNull("size"))
            assertFalse(value.has("uri"))
            assertFalse(value.has("path"))
        }
    }
    @Test fun revokedDocumentAccessIsReportedWithoutStartingTransfer() {
        ActivityScenario.launch(MainActivity::class.java).let { scenario ->
            ready(scenario)
            invoke(scenario, "choose_source")
            device.wait(Until.hasObject(By.pkg("com.google.android.documentsui")), 10000)
            if (device.findObject(By.text("Hermod test documents")) == null) device.findObject(By.desc("Show roots"))?.click()
            check(device.wait(Until.findObject(By.text("Hermod test documents")), 10000) != null)
            device.findObject(By.text("Hermod test documents")).click()
            check(device.wait(Until.findObject(By.text("revoked.txt")), 10000) != null)
            device.findObject(By.text("revoked.txt")).click()
            val failure = result(scenario)
            assertFalse(failure.getBoolean("ok"))
            assertTrue(failure.getString("error").isNotEmpty())
            invoke(scenario, "status")
            assertTrue(result(scenario).isNull("value"))
        }
    }
    @Test fun api37PermissionDenialHasActionableMessage() {
        if (Build.VERSION.SDK_INT < 37) return
        // Orchestrator 为每个测试清除应用数据；不撤销自身进程的权限，避免系统终止测试。
        ActivityScenario.launch(MainActivity::class.java).let { scenario ->
            ready(scenario)
            invoke(scenario, "start_transfer", "{\"sourceId\":null,\"code\":null,\"text\":\"hello\"}")
            device.wait(Until.findObject(By.res("com.android.permissioncontroller", "permission_deny_button")), 10000)?.click()
            val value = result(scenario)
            assertFalse(value.getBoolean("ok"))
            assertTrue(value.getString("error").contains("Local network permission denied"))
        }
    }
    @Test fun foregroundServiceSurvivesRecreationAndNotificationCancels() {
        device.executeShellCommand("pm grant ${context.packageName} android.permission.POST_NOTIFICATIONS")
        if (Build.VERSION.SDK_INT >= 37) device.executeShellCommand("pm grant ${context.packageName} android.permission.ACCESS_LOCAL_NETWORK")
        ActivityScenario.launch(MainActivity::class.java).let { scenario ->
            ready(scenario)
            scenario.onActivity {
                TransferService.cancelRequested = false
                it.startForegroundService(Intent(it, TransferService::class.java))
            }
            Thread.sleep(700)
            scenario.recreate()
            ready(scenario)
            assertFalse(TransferService.cancelRequested)
            TransferService.update("Transferring", 32768, 65536)
            device.openNotification()
            check(device.wait(Until.findObject(By.text("Cancel")), 10000) != null)
            device.findObject(By.text("Cancel")).click()
            repeat(30) { if (!TransferService.cancelRequested) Thread.sleep(100) }
            assertTrue(TransferService.cancelRequested)
            context.stopService(Intent(context, TransferService::class.java))
        }
    }
}
