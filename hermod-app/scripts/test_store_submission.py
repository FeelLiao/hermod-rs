"""离线验证商店自动提交边界：已有草稿、版本回退、失败传播与日志脱敏。"""
import contextlib
import copy
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
import zipfile
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("submit_store", Path(__file__).with_name("submit-store.py"))
store = importlib.util.module_from_spec(spec)
spec.loader.exec_module(store)

SECRETS = {
    "AZURE_AD_TENANT_ID": "11111111-1111-1111-1111-111111111111",
    "AZURE_AD_APPLICATION_CLIENT_ID": "22222222-2222-2222-2222-222222222222",
    "AZURE_AD_APPLICATION_SECRET": "fake-secret-with-$()-and-quote'",
    "SELLER_ID": "12345678",
    "MS_STORE_APP_ID": "9PMPWN3HMR3M",
}


class FakeCli:
    def __init__(self):
        self.calls = []
        self.app = {
            "id": SECRETS["MS_STORE_APP_ID"],
            "packageidentityname": store.PACKAGE_IDENTITY,
            "packagefamilyname": store.PACKAGE_FAMILY,
            "pendingapplicationsubmission": None,
            "lastpublishedapplicationsubmission": {"id": "100"},
        }
        self.published = {"id": "100", "applicationpackages": [
            {"version": "1.4.3.0", "filestatus": "Uploaded"},
        ]}
        self.submitted = {"id": "101", "status": "PreProcessing", "statusdetails": {"errors": []}}
        self.upload_error = None
        self.did_publish = False

    def call(self, *arguments, **options):
        self.calls.append(arguments)
        if arguments[:2] == ("apps", "get"):
            return copy.deepcopy(self.app)
        if arguments[:2] == ("submission", "get"):
            return copy.deepcopy(self.submitted if self.did_publish else self.published)
        if arguments[0] == "publish":
            if self.upload_error:
                raise self.upload_error
            self.did_publish = True


class StoreSubmissionTests(unittest.TestCase):
    def run_submit(self, cli):
        with contextlib.redirect_stdout(io.StringIO()):
            return store.submit(cli, SECRETS, Path("/tmp/verified/hermod-windows.msixbundle"), "1.4.4.0")

    def assert_not_published(self, cli, message):
        with self.assertRaisesRegex(store.SubmissionError, message):
            self.run_submit(cli)
        self.assertFalse(cli.did_publish)
        self.assertFalse(any("delete" in command for command in cli.calls))

    def test_publishes_bundle_and_reports_acceptance_without_overriding_store_settings(self):
        cli = FakeCli()
        self.assertEqual(self.run_submit(cli), {"submission_id": "101", "status": "PreProcessing", "version": "1.4.4.0"})
        self.assertEqual(cli.calls[0][0], "reconfigure")
        self.assertEqual(cli.calls[1], ("settings", "--enableTelemetry", "false"))
        publish = next(command for command in cli.calls if command[0] == "publish")
        self.assertEqual(publish, ("publish", "/tmp/verified/hermod-windows.msixbundle", "--appId", SECRETS["MS_STORE_APP_ID"]))

    def test_existing_draft_or_review_is_preserved(self):
        cli = FakeCli()
        cli.app["pendingapplicationsubmission"] = {"id": "200"}
        self.assert_not_published(cli, "unfinished Store submission")

    def test_wrong_product_or_package_identity_never_uploads(self):
        for key in ("id", "packageidentityname", "packagefamilyname"):
            with self.subTest(key=key):
                cli = FakeCli()
                cli.app[key] = "wrong"
                self.assert_not_published(cli, "does not match")

    def test_first_publication_and_concurrent_draft_are_rejected(self):
        cli = FakeCli()
        cli.app["lastpublishedapplicationsubmission"] = None
        self.assert_not_published(cli, "first publication")
        cli = FakeCli()
        cli.published["id"] = "200"
        self.assert_not_published(cli, "changed during preflight")

    def test_repeated_or_older_versions_do_not_create_submissions(self):
        for version in ("1.4.4.0", "1.4.10.0", "malformed"):
            with self.subTest(version=version):
                cli = FakeCli()
                cli.published["applicationpackages"][0]["version"] = version
                self.assert_not_published(cli, "version")

    def test_upload_failure_is_not_swallowed(self):
        cli = FakeCli()
        cli.upload_error = store.SubmissionError("Upload failed")
        self.assert_not_published(cli, "Upload failed")

    def test_post_commit_failures_and_missing_new_submission_fail_the_job(self):
        for change in ({"status": "CommitFailed"}, {"status": "PendingCommit"}, {"id": "100"},
                       {"statusdetails": {"errors": [{"code": "InvalidPackage"}]}}):
            with self.subTest(change=change):
                cli = FakeCli()
                cli.submitted.update(change)
                with self.assertRaises(store.SubmissionError):
                    self.run_submit(cli)
                self.assertTrue(cli.did_publish)

    def test_authentication_failure_stops_before_reading_or_changing_the_app(self):
        cli = FakeCli()
        with patch.object(cli, "call", side_effect=store.SubmissionError("Authentication failed")) as call:
            with self.assertRaisesRegex(store.SubmissionError, "Authentication failed"):
                self.run_submit(cli)
        self.assertEqual(call.call_count, 1)
        self.assertEqual(call.call_args.args[0], "reconfigure")
        self.assertFalse(cli.did_publish)

    def test_secret_validation_reports_names_only(self):
        for name in store.REQUIRED_SECRETS:
            values = dict(SECRETS, **{name: " "})
            with self.assertRaisesRegex(store.SubmissionError, name) as error:
                store.credentials(values)
            self.assertNotIn(SECRETS["AZURE_AD_APPLICATION_SECRET"], str(error.exception))
        with self.assertRaises(store.SubmissionError):
            store.credentials(dict(SECRETS, MS_STORE_APP_ID="not-a-product-id"))

    def test_cli_uses_argument_arrays_and_keeps_json_out_of_logs(self):
        cli = store.StoreCli(SECRETS)
        payload = {"Id": "100", "FileUploadUrl": "https://storage.example/upload?sig=private", "StatusDetails": {"Errors": []}}
        response = subprocess.CompletedProcess([], 0, json.dumps(payload), "Retrieving Submission...")
        with patch.object(store.subprocess, "run", return_value=response) as run:
            result = cli.call("submission", "get", SECRETS["MS_STORE_APP_ID"], json_output=True)
        self.assertEqual(result["statusdetails"], {"errors": []})
        self.assertIsInstance(run.call_args.args[0], list)
        self.assertFalse(run.call_args.kwargs.get("shell", False))
        self.assertEqual(run.call_args.kwargs["env"]["MSSTORE_OUTPUT_STREAM"], "stderr")

    def test_cli_failures_redact_secrets_and_sas_and_timeouts_hide_arguments(self):
        cli = store.StoreCli(SECRETS)
        diagnostic = f"::error::{SECRETS['AZURE_AD_APPLICATION_SECRET']} https://storage.example/file?sig=hidden-token Bearer hidden-bearer"
        with patch.object(store.subprocess, "run", return_value=subprocess.CompletedProcess([], 1, "", diagnostic)):
            with self.assertRaises(store.SubmissionError) as error:
                cli.call("publish", "bundle.msixbundle")
        for value in (SECRETS["AZURE_AD_APPLICATION_SECRET"], "hidden-token", "hidden-bearer"):
            self.assertNotIn(value, str(error.exception))
        self.assertIn("Store CLI: ::error::", str(error.exception))
        with patch.object(store.subprocess, "run", side_effect=subprocess.TimeoutExpired(["secret-argument"], 1)):
            with self.assertRaises(store.SubmissionError) as error:
                cli.call("reconfigure", "--clientSecret", SECRETS["AZURE_AD_APPLICATION_SECRET"])
        self.assertNotIn("secret-argument", str(error.exception))

    def test_invalid_cli_json_does_not_echo_the_response(self):
        cli = store.StoreCli(SECRETS)
        for payload in (SECRETS["AZURE_AD_APPLICATION_SECRET"], "null", "[]"):
            with self.subTest(payload=payload):
                with patch.object(store.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, payload, "")):
                    with self.assertRaises(store.SubmissionError) as error:
                        cli.call("submission", "get", SECRETS["MS_STORE_APP_ID"], json_output=True)
                self.assertNotIn(SECRETS["AZURE_AD_APPLICATION_SECRET"], str(error.exception))

    def test_invalid_bundle_fails_before_any_cli_or_authentication_call(self):
        with tempfile.TemporaryDirectory() as directory:
            bundle = Path(directory) / "hermod-windows.msixbundle"
            bundle.write_bytes(b"not an MSIXBundle")
            with patch.dict(os.environ, SECRETS), patch.object(store.sys, "argv", ["submit-store.py", str(bundle)]), \
                    patch.object(store.StoreCli, "call") as call:
                with self.assertRaises(zipfile.BadZipFile):
                    store.main()
                call.assert_not_called()


if __name__ == "__main__":
    unittest.main()
