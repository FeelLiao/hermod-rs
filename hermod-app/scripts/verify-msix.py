#!/usr/bin/env python3
"""核对上传商店的未签名包、架构和双语资源；不读取任何签名私钥。"""
import json
import pathlib
import shutil
import struct
import sys
import tempfile
import xml.etree.ElementTree as ET
import zipfile

IDENTITY = "FeelLiao.HermodTransfer"
PUBLISHER = "CN=37A01BFF-1DAB-45E5-9B67-AA76C0655A7C"
DISPLAY_NAME = "HermodTransfer"
NS = {"f": "http://schemas.microsoft.com/appx/manifest/foundation/windows10",
      "uap": "http://schemas.microsoft.com/appx/manifest/uap/windows10"}
BUNDLE_NS = {"b": "http://schemas.microsoft.com/appx/2013/bundle"}


def check(condition, message):
    if not condition:
        raise ValueError(message)


def verify(path, architecture, version):
    check(architecture in ("x64", "arm64"), "Unsupported MSIX architecture")
    major, minor, patch = map(int, version.split("."))
    with zipfile.ZipFile(path) as package:
        names = package.namelist()
        check(len(names) == len(set(names)), "Duplicate MSIX entries")
        check("AppxSignature.p7x" not in names, "Store upload must remain unsigned")
        check(not any(pathlib.PurePosixPath(name).suffix.lower() in
                      (".pfx", ".key", ".pem", ".cer", ".pdb", ".log", ".ps1")
                      for name in names), "Unexpected signing/debug/helper file in MSIX")
        manifest = ET.fromstring(package.read("AppxManifest.xml"))
        identity = manifest.find("f:Identity", NS)
        check(identity is not None, "Missing package identity")
        check(identity.get("Name") == IDENTITY, "Incorrect Store identity name")
        check(identity.get("Publisher") == PUBLISHER, "Incorrect Store publisher")
        check(identity.get("Version") == f"{major + 1}.{minor}.{patch}.0", "Incorrect Store version")
        check(identity.get("ProcessorArchitecture") == architecture, "Incorrect MSIX architecture")
        check(manifest.findtext("f:Properties/f:DisplayName", namespaces=NS) == DISPLAY_NAME,
              "Package display name must match the reserved Store name HermodTransfer")
        check(manifest.findtext("f:Properties/f:PublisherDisplayName", namespaces=NS) == "FeelLiao",
              "Incorrect publisher display name")
        languages = [item.get("Language") for item in manifest.findall("f:Resources/f:Resource", NS)]
        check(languages == ["en-US", "zh-CN"], "Expected English and Simplified Chinese resources")
        check(manifest.findtext("f:Properties/f:Description", namespaces=NS) == "ms-resource:AppDescription",
              "Missing localized package description")
        visual = manifest.find("f:Applications/f:Application/uap:VisualElements", NS)
        check(visual is not None and visual.get("DisplayName") == DISPLAY_NAME,
              "Application display name must match the reserved Store name HermodTransfer")
        check(visual is not None and visual.get("Description") == "ms-resource:AppDescription",
              "Missing localized application description")
        descriptions = []
        for language in languages:
            strings = ET.fromstring(package.read(f"Strings/{language}/Resources.resw"))
            description = strings.findtext("data[@name='AppDescription']/value")
            check(bool(description), f"Missing description for {language}")
            descriptions.append(description)
        check(descriptions[0] != descriptions[1], "Description was not translated")
        check(package.getinfo("resources.pri").file_size > 0, "Missing compiled language resources")
        with package.open("hermod-app.exe") as binary:
            header = binary.read(4096)
        check(header[:2] == b"MZ" and len(header) >= 64, "Invalid Windows executable")
        pe = struct.unpack_from("<I", header, 60)[0]
        check(header[pe:pe + 4] == b"PE\0\0", "Invalid PE header")
        machine = struct.unpack_from("<H", header, pe + 4)[0]
        check(machine == {"x64": 0x8664, "arm64": 0xAA64}[architecture], "Wrong executable architecture")


def verify_bundle(path, version):
    major, minor, patch = map(int, version.split("."))
    expected_version = f"{major + 1}.{minor}.{patch}.0"
    packages = {arch: f"hermod-windows-{arch}.msix" for arch in ("x64", "arm64")}
    metadata = {"AppxMetadata/AppxBundleManifest.xml", "AppxBlockMap.xml", "[Content_Types].xml"}
    with zipfile.ZipFile(path) as bundle:
        names = bundle.namelist()
        check(len(names) == len(set(names)), "Duplicate MSIXBundle entries")
        check(set(names) == metadata | set(packages.values()),
              "Bundle must contain only metadata and unsigned x64/ARM64 MSIX packages")
        manifest = ET.fromstring(bundle.read("AppxMetadata/AppxBundleManifest.xml"))
        identity = manifest.find("b:Identity", BUNDLE_NS)
        check(identity is not None, "Missing bundle identity")
        check(identity.get("Name") == IDENTITY and identity.get("Publisher") == PUBLISHER,
              "Incorrect Store bundle identity")
        check(identity.get("Version") == expected_version, "Incorrect Store bundle version")
        entries = manifest.findall("b:Packages/b:Package", BUNDLE_NS)
        check(len(entries) == 2 and {item.get("Architecture") for item in entries} == set(packages),
              "Bundle must declare exactly one x64 and one ARM64 application")
        for entry in entries:
            architecture = entry.get("Architecture")
            name = packages[architecture]
            check(entry.get("Type") == "application" and entry.get("FileName") == name,
                  "Incorrect bundle application entry")
            check(entry.get("Version") == expected_version, "Mismatched bundled package version")
            check(entry.get("Size") == str(bundle.getinfo(name).file_size), "Incorrect bundled package size")
            # 有界复制到临时文件，避免把两个完整安装包同时加载进内存。
            with bundle.open(name) as content, tempfile.TemporaryFile() as package:
                shutil.copyfileobj(content, package, length=1024 * 1024)
                package.seek(0)
                verify(package, architecture, version)


if __name__ == "__main__":
    config = pathlib.Path(__file__).resolve().parents[1] / "src-tauri/tauri.conf.json"
    version = json.loads(config.read_text())["version"]
    if sys.argv[2] == "--bundle":
        verify_bundle(sys.argv[1], version)
        print("Store MSIXBundle verified: x64 and ARM64 packages, identity, reserved name, version, languages, no signatures")
    else:
        verify(sys.argv[1], sys.argv[2], version)
        print("Store MSIX verified: identity, reserved name, version, architecture, en-US/zh-CN resources, no signature")
