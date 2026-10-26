#!/usr/bin/env python3
"""核对上传商店的未签名包、架构和双语资源；不读取任何签名私钥。"""
import json
import pathlib
import struct
import sys
import xml.etree.ElementTree as ET
import zipfile

IDENTITY = "FeelLiao.HermodTransfer"
PUBLISHER = "CN=37A01BFF-1DAB-45E5-9B67-AA76C0655A7C"
NS = {"f": "http://schemas.microsoft.com/appx/manifest/foundation/windows10",
      "uap": "http://schemas.microsoft.com/appx/manifest/uap/windows10"}


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
        check(manifest.findtext("f:Properties/f:PublisherDisplayName", namespaces=NS) == "FeelLiao",
              "Incorrect publisher display name")
        languages = [item.get("Language") for item in manifest.findall("f:Resources/f:Resource", NS)]
        check(languages == ["en-US", "zh-CN"], "Expected English and Simplified Chinese resources")
        check(manifest.findtext("f:Properties/f:Description", namespaces=NS) == "ms-resource:AppDescription",
              "Missing localized package description")
        visual = manifest.find("f:Applications/f:Application/uap:VisualElements", NS)
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


if __name__ == "__main__":
    config = pathlib.Path(__file__).resolve().parents[1] / "src-tauri/tauri.conf.json"
    verify(sys.argv[1], sys.argv[2], json.loads(config.read_text())["version"])
    print("Store MSIX verified: identity, version, architecture, en-US/zh-CN resources, no signature")
