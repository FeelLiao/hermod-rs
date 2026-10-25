#!/usr/bin/env python3
"""检查 APK 中每个原生库的 ABI、ELF 对齐和 ZIP 压缩方式。"""
import pathlib
import struct
import sys
import zipfile

with zipfile.ZipFile(sys.argv[1]) as apk:
    libraries = [p for p in apk.infolist() if p.filename.startswith('lib/') and p.filename.endswith('.so')]
    assert libraries, 'Missing native libraries'
    assert any(p.filename.endswith('/libhermod_app_lib.so') for p in libraries)
    for entry in libraries:
        assert pathlib.PurePosixPath(entry.filename).parts[1] == 'arm64-v8a', entry.filename
        assert entry.compress_type == zipfile.ZIP_STORED, 'Native libraries must load read-only from APK'
        data = apk.read(entry)
        assert data[:6] == b'\x7fELF\x02\x01', 'Expected ELF64 little endian'
        assert struct.unpack_from('<H', data, 18)[0] == 183, 'Expected AArch64'
        phoff = struct.unpack_from('<Q', data, 32)[0]
        entsize, count = struct.unpack_from('<HH', data, 54)
        for index in range(count):
            off = phoff + index * entsize
            if struct.unpack_from('<I', data, off)[0] == 1:
                alignment = struct.unpack_from('<Q', data, off + 48)[0]
                assert alignment >= 16384, f'{entry.filename}: ELF segment alignment {alignment}'
print('APK contains only ARM64, uncompressed, 16 KB aligned native libraries')
