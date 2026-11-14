"""Source-only Android guardrails; does not invoke Gradle, Cargo, Flutter or SDK tools."""
import pathlib
import re
import unittest
import xml.etree.ElementTree as ET

ROOT = pathlib.Path(__file__).resolve().parents[2]
ANDROID = ROOT / "flutter/android"
NS = "{http://schemas.android.com/apk/res/android}"


class AndroidContractTests(unittest.TestCase):
    def test_manifest_permissions_and_service(self):
        root = ET.parse(ANDROID / "app/src/main/AndroidManifest.xml").getroot()
        permissions = {node.get(NS + "name") for node in root.findall("uses-permission")}
        self.assertEqual(permissions, {
            "android.permission.INTERNET", "android.permission.WAKE_LOCK", "android.permission.POST_NOTIFICATIONS",
            "android.permission.FOREGROUND_SERVICE", "android.permission.FOREGROUND_SERVICE_DATA_SYNC",
        })
        app = root.find("application")
        self.assertEqual(app.get(NS + "name"), ".HermodApplication")
        self.assertEqual(app.get(NS + "allowBackup"), "false")
        service = app.find("service")
        self.assertEqual(service.get(NS + "foregroundServiceType"), "dataSync")
        self.assertEqual(service.get(NS + "exported"), "false")

    def test_xml_resources_parse(self):
        for path in (ANDROID / "app/src/main/res").rglob("*.xml"):
            ET.parse(path)

    def test_fixed_build_and_no_debug_fallback(self):
        gradle = (ANDROID / "app/build.gradle.kts").read_text(encoding="utf-8")
        script = (ROOT / "flutter/scripts/build-android.sh").read_text(encoding="utf-8")
        self.assertIn('ndkVersion = "28.2.13676358"', gradle)
        self.assertIn("ndk_version=28.2.13676358", script)
        self.assertNotIn('getByName("debug")', gradle)
        for abi in ("arm64-v8a", "x86_64"):
            self.assertIn(abi, script)
            self.assertIn(abi, gradle)
        self.assertIn('GITHUB_ACTIONS', script)
        self.assertIn('libhermod_flutter_bridge.so', script)
        self.assertIn('2\\.13\\.0', script)
        blocks = re.findall(r"<<'PY'\n(.*?)\nPY", script, re.S)
        self.assertEqual(len(blocks), 3)
        for block in blocks:
            compile(block, "build-android embedded Python", "exec")

    def test_native_channel_and_stream_contract(self):
        source = (ANDROID / "app/src/main/kotlin/org/hermod/transfer/MainActivity.kt").read_text(encoding="utf-8")
        for method in ("pickSource", "exportFile", "startTransferService"):
            self.assertIn('"' + method + '"', source)
        self.assertIn('"hermod/platform"', (ANDROID / "app/src/main/kotlin/org/hermod/transfer/HermodApplication.kt").read_text(encoding="utf-8"))
        self.assertIn('mapOf("path" to target.canonicalPath, "name" to name)', source)
        self.assertIn('createNewFile()', source)
        self.assertIn('ByteArray(64 * 1024)', source)
        self.assertNotIn('readBytes(', source)
        self.assertNotIn('uri.path', source)

    def test_ready_handshake_and_id_scoped_stop(self):
        native = ANDROID / "app/src/main/kotlin/org/hermod/transfer"
        app = (native / "HermodApplication.kt").read_text(encoding="utf-8")
        service = (native / "TransferService.kt").read_text(encoding="utf-8")
        activity = (native / "MainActivity.kt").read_text(encoding="utf-8")
        self.assertIn('app.startTransfer(id)', activity)
        self.assertNotIn('finishResult(result, true)', activity)
        self.assertIn('transferId != id', app)
        self.assertIn('callback?.invoke(true)', app)
        self.assertIn('serviceReady(this, id)', service)
        self.assertIn('mapOf("operationId" to id, "reason" to reason)', app)
        self.assertIn('stopWaiters', app)
        self.assertIn('"releaseStagedSource"', app)
        self.assertIn('canonical.parentFile == root', app)
        self.assertIn('isSymbolicLink', app)
        self.assertIn('hermod.pickerOutstanding', activity)
        self.assertNotIn('setMethodCallHandler(null)', activity)
        self.assertNotIn('super.configureFlutterEngine', activity)
        self.assertNotIn('takePersistableUriPermission', activity)
        self.assertIn('deadline.postDelayed(expire', service)

    def test_detached_activity_still_routes_process_cleanup(self):
        native = ANDROID / "app/src/main/kotlin/org/hermod/transfer"
        app = (native / "HermodApplication.kt").read_text(encoding="utf-8")
        activity = (native / "MainActivity.kt").read_text(encoding="utf-8")
        # Structural guard only: cleanup dispatch precedes the host-null/UI branch.
        dispatch = app[app.index('channel.setMethodCallHandler'):app.index('fun attach(')]
        host_gate = dispatch.index('val host = activity')
        for method in ('stopTransferService', 'releaseStagedSource', 'getSupportDirectory'):
            self.assertLess(dispatch.index('"' + method + '"'), host_gate)
            self.assertNotIn('"' + method + '"', activity)
        self.assertIn('canonical.path !in exportSources', app)
        self.assertIn('app.exportSources.add(canonical.path)', activity)
        self.assertIn('app.exportSources.remove(source.path)', activity)

    def test_kotlin_opt_out_has_explicit_plugin(self):
        properties = (ANDROID / "gradle.properties").read_text(encoding="utf-8")
        app = (ANDROID / "app/build.gradle.kts").read_text(encoding="utf-8")
        self.assertIn('android.builtInKotlin=false', properties)
        self.assertIn('id("org.jetbrains.kotlin.android")', app)
        self.assertLess(app.index('id("org.jetbrains.kotlin.android")'), app.index('id("dev.flutter.flutter-gradle-plugin")'))

    def test_workflow_unsigned_explicit_and_secrets_only(self):
        source = (ROOT / ".github/workflows/android-release.yml").read_text(encoding="utf-8")
        for name in ("ANDROID_KEYSTORE_BASE64", "ANDROID_KEYSTORE_PASSWORD", "ANDROID_KEY_ALIAS", "ANDROID_KEY_PASSWORD"):
            self.assertIn("secrets." + name, source)
        self.assertIn("android-UNSIGNED-inspection-only", source)
        self.assertIn("github.event_name == 'workflow_dispatch' && inputs.unsigned", source)
        self.assertNotIn("contents: write", source)


if __name__ == "__main__":
    unittest.main()
