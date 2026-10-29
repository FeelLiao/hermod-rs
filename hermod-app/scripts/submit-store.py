#!/usr/bin/env python3
"""在临时 CI runner 上用官方 Store CLI 更新已发布应用；不输出凭据或完整提交 JSON。"""
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import zipfile

REQUIRED_SECRETS = (
    "AZURE_AD_TENANT_ID", "AZURE_AD_APPLICATION_CLIENT_ID",
    "AZURE_AD_APPLICATION_SECRET", "SELLER_ID", "MS_STORE_APP_ID",
)
PACKAGE_IDENTITY = "FeelLiao.HermodTransfer"
PACKAGE_FAMILY = "FeelLiao.HermodTransfer_8qnpvd7fj1r9a"
ACCEPTED_STATES = {"PreProcessing", "Certification", "Release", "PendingPublication", "Publishing", "Published"}


class SubmissionError(RuntimeError):
    pass


def credentials(environment):
    missing = [name for name in REQUIRED_SECRETS if not environment.get(name, "").strip()]
    if missing:
        raise SubmissionError("Missing Actions secrets: " + ", ".join(missing))
    values = {name: environment[name] for name in REQUIRED_SECRETS}
    if not re.fullmatch(r"[A-Z0-9]{12}", values["MS_STORE_APP_ID"]):
        raise SubmissionError("MS_STORE_APP_ID must be the 12-character Store product ID")
    return values


def version_tuple(value):
    if not isinstance(value, str) or not re.fullmatch(r"\d+\.\d+\.\d+\.\d+", value):
        raise SubmissionError("Store returned an invalid package version")
    return tuple(map(int, value.split(".")))


class StoreCli:
    def __init__(self, environment):
        self.environment = dict(environment, MSSTORE_OUTPUT_STREAM="stderr", NO_COLOR="1")
        self.secrets = [environment[name] for name in REQUIRED_SECRETS if environment.get(name)]

    def redact(self, message):
        for value in sorted(self.secrets, key=len, reverse=True):
            message = message.replace(value, "***")
        # 失败诊断可能包含上传 SAS URL；不能把临时上传权限写进 Actions 日志。
        message = re.sub(r"(https?://[^\s<>\"'?]+)\?[^\s<>\"']+", r"\1?<redacted>", message)
        return re.sub(r"(?i)Bearer\s+\S+", "Bearer <redacted>", message)

    def call(self, *arguments, json_output=False, timeout=120):
        operation = " ".join(arguments[:2]) if arguments[0] in ("apps", "submission") else arguments[0]
        try:
            result = subprocess.run(
                ["msstore", *arguments], env=self.environment, stdin=subprocess.DEVNULL,
                capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout,
            )
        except subprocess.TimeoutExpired:
            raise SubmissionError(f"Store CLI {operation} timed out; inspect Partner Center before retrying") from None
        except OSError:
            raise SubmissionError("Could not run the pinned Microsoft Store CLI") from None
        if result.returncode:
            details = self.redact(result.stderr)[-3000:]
            # 每行加前缀，避免外部诊断被解释成 GitHub 工作流控制命令。
            details = "\n".join("Store CLI: " + line for line in details.splitlines())
            raise SubmissionError(f"Store CLI {operation} failed (exit {result.returncode})\n{details}")
        if json_output:
            try:
                value = json.loads(result.stdout, object_hook=lambda data: {key.lower(): item for key, item in data.items()})
            except (ValueError, TypeError):
                raise SubmissionError(f"Store CLI {operation} did not return valid JSON") from None
            if not isinstance(value, dict):
                raise SubmissionError(f"Store CLI {operation} returned an unexpected response")
            return value
        return None


def configured_app(cli, values):
    app_id = values["MS_STORE_APP_ID"]
    cli.call(
        "reconfigure", "--tenantId", values["AZURE_AD_TENANT_ID"],
        "--sellerId", values["SELLER_ID"],
        "--clientId", values["AZURE_AD_APPLICATION_CLIENT_ID"],
        "--clientSecret", values["AZURE_AD_APPLICATION_SECRET"],
    )
    # settings 也要求已经配置凭据，因此必须先完成非交互 reconfigure。
    cli.call("settings", "--enableTelemetry", "false")
    app = cli.call("apps", "get", app_id, json_output=True)
    if (app.get("id") != app_id or app.get("packageidentityname") != PACKAGE_IDENTITY
            or app.get("packagefamilyname") != PACKAGE_FAMILY):
        raise SubmissionError("Store product does not match the HermodTransfer package identity")
    return app


def reference_id(app, key, label):
    reference = app.get(key)
    if reference is None:
        return None
    if not isinstance(reference, dict):
        raise SubmissionError(f"Store returned an invalid {label} submission reference")
    value = reference.get("id")
    if value is not None and (not isinstance(value, str) or not re.fullmatch(r"[0-9]+", value)):
        raise SubmissionError(f"Store returned an invalid {label} submission ID; inspect Partner Center")
    return value


def submission_status(submission):
    # 状态只允许短字母串；不把意外的 URL、JSON 或控制字符带入日志。
    status = submission.get("status")
    return status if isinstance(status, str) and re.fullmatch(r"[A-Za-z]{1,40}", status) else "Unknown"


def diagnose(cli, values):
    app = configured_app(cli, values)
    pending_id = reference_id(app, "pendingapplicationsubmission", "pending")
    published_id = reference_id(app, "lastpublishedapplicationsubmission", "published")
    reference = "present" if app.get("pendingapplicationsubmission") is not None else "absent"
    lines = [
        "Microsoft Store read-only diagnostics",
        f"Last published submission ID: {published_id or 'none'}",
        f"Pending submission reference: {reference}",
        f"Pending submission ID: {pending_id or 'none'}",
    ]
    expected_id = pending_id or published_id
    if expected_id:
        current = cli.call("submission", "get", values["MS_STORE_APP_ID"], json_output=True)
        if current.get("id") != expected_id:
            raise SubmissionError("Store submission changed during diagnostics; rerun diagnostics")
        status = submission_status(current)
        lines.append(f"{'Pending' if pending_id else 'Published'} submission status: {status}")
        if pending_id and status == "PendingCommit":
            lines.append("PendingCommit is an unsubmitted draft, not a submission under review.")
    if not pending_id:
        lines.append("No pending submission ID was returned; there is no pending submission reference to block publication.")
    lines.append("No Store submission was created, changed, deleted or uploaded by this diagnostic run.")
    return "\n".join(lines)


def submit(cli, values, bundle, version):
    app_id = values["MS_STORE_APP_ID"]
    app = configured_app(cli, values)
    # CLI 以非 null 的 ID 判断待处理提交，不能仅凭非空对象把占位字段误判为草稿。
    pending_id = reference_id(app, "pendingapplicationsubmission", "pending")
    if pending_id:
        current = cli.call("submission", "get", app_id, json_output=True)
        if current.get("id") != pending_id:
            raise SubmissionError("Store submission changed during preflight; retry after checking Partner Center")
        status = submission_status(current)
        if status == "PendingCommit":
            detail = "This is an unsubmitted draft, not a submission under review."
        else:
            detail = "Inspect its processing state in Partner Center."
        # 官方 publish 会删除草稿；即使尚未送审，也不能自动丢弃其中的编辑。
        raise SubmissionError(
            f"Store pending submission {pending_id} has status {status}. {detail} "
            "The existing submission was preserved and no package was uploaded. "
            "Resolve it in Partner Center before retrying."
        )
    published_id = reference_id(app, "lastpublishedapplicationsubmission", "published")
    if not published_id:
        raise SubmissionError("The first publication must be completed in Partner Center")
    published = cli.call("submission", "get", app_id, json_output=True)
    if published.get("id") != published_id:
        raise SubmissionError("Store submission changed during preflight; retry after resolving the pending submission")
    packages = [p for p in published.get("applicationpackages", []) if p.get("filestatus") != "PendingDelete"]
    if not packages or version_tuple(version) <= max(version_tuple(p.get("version")) for p in packages):
        raise SubmissionError("The bundle version must be newer than every published package; bump the application version")

    print(f"Submitting verified Windows bundle version {version} to Microsoft Store...", flush=True)
    # 只更新包；不传 priceId、元数据或发布模式覆盖项。CLI 会提交并轮询初始处理结果。
    cli.call("publish", str(bundle), "--appId", app_id, timeout=1800)
    submission = cli.call("submission", "get", app_id, json_output=True)
    submission_id = submission.get("id", "")
    status = submission.get("status")
    if not re.fullmatch(r"\d+", str(submission_id)) or submission_id == published_id:
        raise SubmissionError("Could not identify the new Store submission; inspect Partner Center")
    if status not in ACCEPTED_STATES or (submission.get("statusdetails") or {}).get("errors"):
        raise SubmissionError(f"Store submission {submission_id} was not accepted; inspect its status in Partner Center")
    return {"submission_id": submission_id, "status": status, "version": version}


def report(message):
    print(message)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as summary:
            summary.write("```text\n" + message + "\n```\n")


def main():
    values = credentials(os.environ)
    if len(sys.argv) != 2:
        raise SubmissionError("Usage: submit-store.py PATH_TO_MSIXBUNDLE | --diagnose")
    if sys.argv[1] == "--diagnose":
        report(diagnose(StoreCli(os.environ), values))
        return
    bundle = Path(sys.argv[1]).resolve(strict=True)
    if bundle.name != "hermod-windows.msixbundle":
        raise SubmissionError("Expected hermod-windows.msixbundle")
    config = Path(__file__).resolve().parents[1] / "src-tauri/tauri.conf.json"
    app_version = json.loads(config.read_text(encoding="utf-8"))["version"]
    spec = importlib.util.spec_from_file_location("verify_msix", Path(__file__).with_name("verify-msix.py"))
    verifier = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(verifier)
    # 在调用认证及提交命令之前再次校验身份、双架构、版本及未签名状态。
    verifier.verify_bundle(bundle, app_version)
    major, minor, patch = map(int, app_version.split("."))
    result = submit(StoreCli(os.environ), values, bundle, f"{major + 1}.{minor}.{patch}.0")
    message = (
        f"Microsoft Store submission {result['submission_id']}: {result['status']} "
        f"(package {result['version']}). Certification and publication continue in Partner Center."
    )
    report(message)


if __name__ == "__main__":
    try:
        main()
    except (SubmissionError, OSError, ValueError, zipfile.BadZipFile) as error:
        print(f"Store submission failed: {error}", file=sys.stderr)
        sys.exit(1)
