#!/usr/bin/env python3
"""验证 Flutter Windows Store 包的身份、架构、内容和本地化。"""
import pathlib
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
        names = set(bundle.namelist())
        require(set(expected.values()).issubset(names), "Bundle must contain x64 and ARM64 packages")
        versions = set()
        for architecture, name in expected.items():
            with bundle.open(name) as source, tempfile.NamedTemporaryFile(suffix=".msix") as package:
                shutil.copyfileobj(source, package)
                package.flush()
                versions.add(verify(package.name, architecture))
        require(len(versions) == 1, "Bundled package versions differ")
        version = versions.pop()
        if expected_version is not None:
            require(version == expected_version, "Bundle version does not match workspace version")
        return version


if __name__ == "__main__":
    verify(sys.argv[1], sys.argv[2])
    print("Flutter Store MSIX verified: identity, architecture, bilingual resources, Core bridge, unsigned payload")
