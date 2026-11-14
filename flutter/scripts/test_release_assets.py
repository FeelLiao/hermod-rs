"""Release dependency/artifact isolation and exact attachment allowlist."""
import contextlib
import io
from pathlib import Path
import runpy
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]


class ReleaseAssetsTests(unittest.TestCase):
    def test_release_waits_for_android_and_reuses_workflow(self):
        release = (ROOT / '.github/workflows/build-release.yml').read_text(encoding='utf-8')
        android = (ROOT / '.github/workflows/android-release.yml').read_text(encoding='utf-8')
        self.assertIn('uses: ./.github/workflows/android-release.yml', release)
        self.assertIn('needs: [verify, linux, windows_bundle, android]', release)
        self.assertIn('workflow_call:', android)
        self.assertNotIn("tags: ['v*']", android)  # no duplicate independent tag build
        self.assertIn('name: release-android\n          path: dist/hermod-android-release.apk', android)
        self.assertIn("if: env.HERMOD_ANDROID_UNSIGNED != 'true'", android)
        self.assertIn("pattern: release-*", release)

    def test_only_three_public_assets_are_allowed(self):
        script = ROOT / 'flutter/scripts/check-release-files.py'
        expected = ('hermod-linux-x64-musl', 'hermod-linux-arm64-musl', 'hermod-android-release.apk')
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in expected:
                (root / name).write_bytes(b'fixture')
            def check():
                with patch.object(sys, 'argv', [str(script), directory]), contextlib.redirect_stdout(io.StringIO()):
                    runpy.run_path(str(script), run_name='__main__')
            check()
            for bad in ('hermod-android-UNSIGNED.apk', 'android-signature.txt', 'hermod-release.jks'):
                path = root / bad
                path.write_bytes(b'fixture')
                with self.subTest(name=bad), self.assertRaises(AssertionError):
                    check()
                path.unlink()
            (root / expected[-1]).unlink()
            with self.assertRaises(AssertionError):
                check()


if __name__ == '__main__':
    unittest.main()
