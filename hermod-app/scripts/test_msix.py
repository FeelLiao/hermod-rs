"""在 Linux 上验证商店包检查器的拒绝路径，不构建或执行 Windows 程序。"""
import importlib.util
import io
from pathlib import Path
import struct
import unittest
import zipfile

spec = importlib.util.spec_from_file_location("verify_msix", Path(__file__).with_name("verify-msix.py"))
verifier = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verifier)
PACKAGING = Path(__file__).resolve().parents[1] / "packaging"


class StorePackageTests(unittest.TestCase):
    def fixture(self, architecture="x64"):
        manifest = (PACKAGING / "AppxManifest.xml").read_text().replace("__VERSION__", "1.4.3.0")
        manifest = manifest.replace('ProcessorArchitecture="x64"', f'ProcessorArchitecture="{architecture}"')
        binary = bytearray(256)
        binary[:2] = b"MZ"
        struct.pack_into("<I", binary, 60, 128)
        binary[128:132] = b"PE\0\0"
        struct.pack_into("<H", binary, 132, {"x64": 0x8664, "arm64": 0xAA64}[architecture])
        return {
            "AppxManifest.xml": manifest.encode(),
            "hermod-app.exe": bytes(binary),
            "resources.pri": b"test resource index",
            **{f"Strings/{language}/Resources.resw": (PACKAGING / "Strings" / language / "Resources.resw").read_bytes()
               for language in ("en-US", "zh-CN")},
        }

    def verify(self, entries, architecture="x64"):
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, "w") as package:
            for name, content in entries.items():
                package.writestr(name, content)
        buffer.seek(0)
        verifier.verify(buffer, architecture, "0.4.3")

    def test_both_architectures_and_real_manifest_resources(self):
        for architecture in ("x64", "arm64"):
            with self.subTest(architecture=architecture):
                self.verify(self.fixture(architecture), architecture)

    def test_rejects_wrong_identity_version_or_language(self):
        for original, changed in [
            (b"FeelLiao.HermodTransfer", b"Hermod.Transfer.Test"),
            (verifier.PUBLISHER.encode(), b"CN=HermodTest"),
            (b"1.4.3.0", b"0.4.3.0"),
            (b"zh-CN", b"fr-FR"),
            (b"ms-resource:AppDescription", b"untranslated"),
        ]:
            with self.subTest(original=original):
                entries = self.fixture()
                entries["AppxManifest.xml"] = entries["AppxManifest.xml"].replace(original, changed)
                with self.assertRaises(ValueError):
                    self.verify(entries)

    def test_rejects_signature_private_material_and_wrong_binary(self):
        for name in ("AppxSignature.p7x", "test.pfx", "Install-Hermod.ps1", "Hermod-Test.cer"):
            with self.subTest(name=name):
                entries = self.fixture()
                entries[name] = b"test"
                with self.assertRaises(ValueError):
                    self.verify(entries)
        entries = self.fixture()
        entries["hermod-app.exe"] = self.fixture("arm64")["hermod-app.exe"]
        with self.assertRaises(ValueError):
            self.verify(entries)

    def test_rejects_missing_localized_resources(self):
        for name in ("resources.pri", "Strings/zh-CN/Resources.resw"):
            with self.subTest(name=name):
                entries = self.fixture()
                del entries[name]
                with self.assertRaises(KeyError):
                    self.verify(entries)


if __name__ == "__main__":
    unittest.main()
