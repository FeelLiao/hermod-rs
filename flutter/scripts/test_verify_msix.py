"""Real ZIP/PE fixture checks, including Windows open-temporary-file semantics."""
import importlib.util
import io
import pathlib
import struct
import unittest
import zipfile

spec = importlib.util.spec_from_file_location('verify_msix', pathlib.Path(__file__).with_name('verify-msix.py'))
verify = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verify)


def package(arch, version='1.5.0.0'):
    output = io.BytesIO()
    pe = bytearray(512)
    pe[:2] = b'MZ'
    struct.pack_into('<I', pe, 60, 128)
    struct.pack_into('<H', pe, 132, {'x64': 0x8664, 'arm64': 0xAA64}[arch])
    manifest = f'''<Package xmlns="{verify.NS['f']}" xmlns:uap="{verify.NS['uap']}" xmlns:uap10="http://schemas.microsoft.com/appx/manifest/uap/windows10/10">
<Identity Name="{verify.IDENTITY}" Publisher="{verify.PUBLISHER}" ProcessorArchitecture="{arch}" Version="{version}"/>
<Applications><Application Executable="HermodTransfer.exe" uap10:RuntimeBehavior="packagedClassicApp" uap10:TrustLevel="appContainer"><uap:VisualElements DisplayName="HermodTransfer"/></Application></Applications></Package>'''
    with zipfile.ZipFile(output, 'w') as archive:
        archive.writestr('AppxManifest.xml', manifest)
        for name in ('HermodTransfer.exe', 'hermod_flutter_bridge.dll'):
            archive.writestr(name, pe)
        for name in ('resources.pri', 'Strings/en-US/Resources.resw', 'Strings/zh-CN/Resources.resw'):
            archive.writestr(name, b'fixture')
    return output.getvalue()


def bundle(arm_version='1.5.0.0', include_arm=True):
    output = io.BytesIO()
    with zipfile.ZipFile(output, 'w') as archive:
        x64 = package('x64')
        arm = package('arm64', arm_version)
        archive.writestr('hermod-windows-x64.msix', x64)
        if include_arm:
            archive.writestr('hermod-windows-arm64.msix', arm)
        archive.writestr('AppxBlockMap.xml', '<BlockMap/>')
        archive.writestr('[Content_Types].xml', '<Types/>')
        archive.writestr('AppxMetadata/AppxBundleManifest.xml', f'''<Bundle xmlns="http://schemas.microsoft.com/appx/2013/bundle"><Identity Name="{verify.IDENTITY}" Publisher="{verify.PUBLISHER}" Version="1.5.0.0"/><Packages><Package Type="application" Architecture="x64" FileName="hermod-windows-x64.msix" Version="1.5.0.0" Size="{len(x64)}"/><Package Type="application" Architecture="arm64" FileName="hermod-windows-arm64.msix" Version="1.5.0.0" Size="{len(arm)}"/></Packages></Bundle>''')
    output.seek(0)
    return output


class BundleTests(unittest.TestCase):
    def test_open_handle_bundle_validation(self):
        self.assertEqual(verify.verify_bundle(bundle(), '1.5.0.0'), '1.5.0.0')

    def test_duplicate_or_signed_bundle_rejected(self):
        output = bundle()
        with zipfile.ZipFile(output, 'a') as archive:
            archive.writestr('AppxSignature.p7x', b'not allowed in Store upload')
        output.seek(0)
        with self.assertRaisesRegex(ValueError, 'Unexpected bundle contents'):
            verify.verify_bundle(output)

    def test_wrong_bundle_publisher_rejected(self):
        original = bundle()
        output = io.BytesIO()
        with zipfile.ZipFile(original) as source, zipfile.ZipFile(output, 'w') as target:
            for name in source.namelist():
                content = source.read(name)
                if name == 'AppxMetadata/AppxBundleManifest.xml':
                    content = content.replace(verify.PUBLISHER.encode(), b'CN=WrongPublisher')
                target.writestr(name, content)
        output.seek(0)
        with self.assertRaisesRegex(ValueError, 'Incorrect bundle identity'):
            verify.verify_bundle(output)

    def test_missing_architecture_rejected(self):
        with self.assertRaisesRegex(ValueError, 'contain x64 and ARM64'):
            verify.verify_bundle(bundle(include_arm=False))

    def test_mismatched_versions_rejected(self):
        with self.assertRaisesRegex(ValueError, 'versions differ'):
            verify.verify_bundle(bundle(arm_version='1.4.5.0'))

    def test_unexpected_release_version_rejected(self):
        with self.assertRaisesRegex(ValueError, 'workspace version'):
            verify.verify_bundle(bundle(), '1.6.0.0')


if __name__ == '__main__':
    unittest.main()
