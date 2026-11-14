#!/usr/bin/env python3
"""验证 Flutter Windows Store 包的身份、架构、内容和本地化。"""
import pathlib
import re
import shutil
import struct
import tempfile
import sys
import xml.etree.ElementTree as ET
import zipfile

NS = {"f": "http://schemas.microsoft.com/appx/manifest/foundation/windows10", "uap": "http://schemas.microsoft.com/appx/manifest/uap/windows10"}
IDENTITY = "FeelLiao.HermodTransfer"
PUBLISHER = "CN=37A01BFF-1DAB-45E5-9B67-AA76C0655A7C"


def require(condition, message):
    if not condition:
        raise ValueError(message)


def verify(path, architecture):
    with zipfile.ZipFile(path) as package:
        names = package.namelist()
        require("AppxSignature.p7x" not in names, "Store upload must be unsigned")
        require("HermodTransfer.exe" in names, "Missing Flutter runner")
        require("hermod_flutter_bridge.dll" in names, "Missing Rust Core bridge")
        require("resources.pri" in names, "Missing localized PRI resources")
        require(all(f"Strings/{language}/Resources.resw" in names for language in ("en-US", "zh-CN")), "Missing bilingual resources")
        require(not any(pathlib.PurePosixPath(name).suffix.lower() in (".pdb", ".pfx", ".key", ".pem") for name in names), "Package contains debug or signing secrets")
        manifest = ET.fromstring(package.read("AppxManifest.xml"))
        identity = manifest.find("f:Identity", NS)
        require(identity is not None and identity.get("Name") == IDENTITY, "Incorrect Store identity")
        require(identity.get("Publisher") == PUBLISHER, "Incorrect Store publisher")
        require(identity.get("ProcessorArchitecture") == architecture, "Incorrect package architecture")
        app = manifest.find("f:Applications/f:Application", NS)
        require(app is not None and app.get("Executable") == "HermodTransfer.exe", "Incorrect application executable")
        uap10 = "{http://schemas.microsoft.com/appx/manifest/uap/windows10/10}"
        require(app.get(uap10 + "RuntimeBehavior") == "packagedClassicApp", "Expected packaged desktop activation")
        require(app.get(uap10 + "TrustLevel") == "appContainer", "GUI must run in AppContainer")
        require(not any(node.get("Name") == "runFullTrust" for node in manifest.iter()), "Full Trust capability is forbidden")
        require(not any("fulltrust" in str(value).lower() for node in manifest.iter() for value in node.attrib.values()), "Full Trust entry/helper is forbidden")
        visual = app.find("uap:VisualElements", NS)
        require(visual is not None and visual.get("DisplayName") == "HermodTransfer", "Reserved Store name mismatch")
        expected_machine = {"x64": 0x8664, "arm64": 0xAA64}[architecture]
        for name in ("HermodTransfer.exe", "hermod_flutter_bridge.dll"):
            with package.open(name) as executable:
                header = executable.read(4096)
            require(header[:2] == b"MZ", f"Invalid PE image inside {name}")
            pe = struct.unpack_from("<I", header, 60)[0]
            machine = struct.unpack_from("<H", header, pe + 4)[0]
            require(machine == expected_machine, f"Wrong architecture inside {name}")
        return identity.get("Version")


def verify_bundle(path, expected_version=None):
    expected = {"x64": "hermod-windows-x64.msix", "arm64": "hermod-windows-arm64.msix"}
    with zipfile.ZipFile(path) as bundle:
        entries = bundle.namelist()
        names = set(entries)
        metadata = {"AppxMetadata/AppxBundleManifest.xml", "AppxBlockMap.xml", "[Content_Types].xml"}
        require(len(entries) == len(names), "Duplicate bundle entries")
        require(set(expected.values()).issubset(names), "Bundle must contain x64 and ARM64 packages")
        require(names == metadata | set(expected.values()), "Unexpected bundle contents or signature")
        ns = {"b": "http://schemas.microsoft.com/appx/2013/bundle"}
        manifest = ET.fromstring(bundle.read("AppxMetadata/AppxBundleManifest.xml"))
        identity = manifest.find("b:Identity", ns)
        require(identity is not None and identity.get("Name") == IDENTITY and identity.get("Publisher") == PUBLISHER, "Incorrect bundle identity")
        declarations = manifest.findall("b:Packages/b:Package", ns)
        require(len(declarations) == 2 and {e.get("Architecture") for e in declarations} == set(expected), "Invalid bundle architecture declarations")
        for entry in declarations:
            name = expected[entry.get("Architecture")]
            require(entry.get("Type") == "application" and entry.get("FileName") == name, "Invalid bundle application entry")
            require(entry.get("Size") == str(bundle.getinfo(name).file_size), "Incorrect bundled package size")
            require(entry.get("Version") == identity.get("Version"), "Bundle declaration version mismatch")
        versions = set()
        for architecture, name in expected.items():
            # Pass the open handle, as v0.4.5 did. Reopening a named temporary
            # file by path fails on Windows while its exclusive handle is open.
            with bundle.open(name) as source, tempfile.TemporaryFile() as package:
                shutil.copyfileobj(source, package)
                package.seek(0)
                versions.add(verify(package, architecture))
        require(len(versions) == 1, "Bundled package versions differ")
        version = versions.pop()
        require(identity.get("Version") == version, "Bundle and package version mismatch")
        if expected_version is not None:
            require(version == expected_version, "Bundle version does not match workspace version")
        return version


if __name__ == "__main__":
    if sys.argv[2] == '--bundle':
        # Match the existing release scripts; Python 3.10 on ubuntu-22.04 has
        # no stdlib tomllib. Read the workspace section explicitly as UTF-8.
        cargo = (pathlib.Path(__file__).resolve().parents[2] / 'Cargo.toml').read_text(encoding='utf-8')
        section = re.search(r'^\[workspace\.package\]\s*\n(?P<body>.*?)(?=^\[|\Z)', cargo, re.MULTILINE | re.DOTALL)
        require(section is not None, 'Missing workspace package metadata')
        version = re.search(r'^version\s*=\s*"(\d+)\.(\d+)\.(\d+)"\s*$', section.group('body'), re.MULTILINE)
        require(version is not None, 'Could not read workspace release version')
        major, minor, patch = map(int, version.groups())
        verify_bundle(sys.argv[1], f'{major + 1}.{minor}.{patch}.0')
        print('Store bundle verified: identity, version, exact unsigned x64/ARM64 contents')
    else:
        verify(sys.argv[1], sys.argv[2])
        print("Flutter Store MSIX verified: identity, architecture, bilingual resources, Core bridge, unsigned payload")
