#!/usr/bin/env python3
"""发布文件白名单：多一个或少一个均阻止上传。"""
import pathlib
import sys
expected = {
    'hermod-linux-x64.AppImage',
    'hermod-linux-x64-musl', 'hermod-linux-arm64-musl',
    'hermod-android-arm64.apk',
}
root = pathlib.Path(sys.argv[1])
actual = {p.name for p in root.iterdir() if p.is_file() and not p.is_symlink()}
assert actual == expected and len(list(root.iterdir())) == len(expected), f'Unexpected Release files: extra={actual - expected}, missing={expected - actual}'
assert all((root / name).stat().st_size > 0 for name in expected), 'Empty Release file'
print('Release allowlist passed: 3 Linux programs and 1 Android APK; Windows packages go only to Microsoft Store')
