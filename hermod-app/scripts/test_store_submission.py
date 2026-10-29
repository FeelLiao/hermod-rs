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
        self.published = {"id": "100", "status": "Published", "applicationpackages": [
            {"version": "1.4.3.0", "filestatus": "Uploaded"},
        ]}
        self.submitted = {"id": "101", "status": "PreProcessing", "statusdetails": {"errors": []}}
        self.pending = None
        self.upload_error = None
        self.did_publish = False

    def call(self, *arguments, **options):
        self.calls.append(arguments)
        if arguments[:2] == ("apps", "get"):
            return copy.deepcopy(self.app)
        if arguments[:2] == ("submission", "get"):
            return copy.deepcopy(self.submitted if self.did_publish else self.pending or self.published)
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

    def test_pending_reference_without_an_id_does_not_block_publication(self):
        for pending in ({}, {"id": None, "resourcelocation": None}):
            with self.subTest(pending=pending):
                cli = FakeCli()
                cli.app["pendingapplicationsubmission"] = pending
                self.assertEqual(self.run_submit(cli)["submission_id"], "101")

    def test_existing_draft_or_review_reports_status_and_is_preserved(self):
        for status in ("PendingCommit", "CommitStarted", "Certification", "CommitFailed", "Published"):
            with self.subTest(status=status):
                cli = FakeCli()
                cli.app["pendingapplicationsubmission"] = {"id": "200"}
                cli.pending = {"id": "200", "status": status}
                self.assert_not_published(cli, f"pending submission 200 has status {status}")

    def test_draft_diagnostic_explains_it_has_not_been_submitted_and_hides_upload_url(self):
        cli = FakeCli()
        cli.app["pendingapplicationsubmission"] = {"id": "200"}
        cli.pending = {"id": "200", "status": "PendingCommit", "fileuploadurl": "https://storage.example/?sig=hidden"}
        with self.assertRaises(store.SubmissionError) as error:
            self.run_submit(cli)
        self.assertIn("unsubmitted draft, not a submission under review", str(error.exception))
        self.assertNotIn("hidden", str(error.exception))
        self.assertFalse(cli.did_publish)

    def test_pending_reference_changes_or_invalid_responses_fail_without_upload(self):
        cli = FakeCli()
        cli.app["pendingapplicationsubmission"] = {"id": "200"}
        cli.pending = {"id": "201", "status": "PendingCommit"}
        self.assert_not_published(cli, "changed during preflight")
        for pending in ("invalid", {"id": ""}, {"id": "not-an-id"}):
            with self.subTest(pending=pending):
                cli = FakeCli()
                cli.app["pendingapplicationsubmission"] = pending
                self.assert_not_published(cli, "invalid pending submission")

    def test_diagnostics_distinguish_absent_and_null_id_references_without_publishing(self):
        for pending in (None, {}, {"id": None, "resourcelocation": None}):
            with self.subTest(pending=pending):
                cli = FakeCli()
                cli.app["pendingapplicationsubmission"] = pending
                report = store.diagnose(cli, SECRETS)
                self.assertIn("Pending submission ID: none", report)
                self.assertIn("Published submission status: Published", report)
                self.assertIn(f"Pending submission reference: {'absent' if pending is None else 'present'}", report)
                self.assertFalse(cli.did_publish)
                self.assertEqual([call[:2] for call in cli.calls[2:]], [("apps", "get"), ("submission", "get")])

    def test_diagnostics_report_drafts_without_leaking_upload_urls_or_changing_them(self):
        cli = FakeCli()
        cli.app["pendingapplicationsubmission"] = {"id": "200"}
        cli.pending = {"id": "200", "status": "PendingCommit", "fileuploadurl": "https://storage.example/?sig=hidden"}
        report = store.diagnose(cli, SECRETS)
        self.assertIn("Pending submission ID: 200", report)
        self.assertIn("Pending submission status: PendingCommit", report)
        for private in (*SECRETS.values(), "hidden", "storage.example"):
            self.assertNotIn(private, report)
        self.assertEqual([call[:2] for call in cli.calls[2:]], [("apps", "get"), ("submission", "get")])
        cli.pending["id"] = "201"
        with self.assertRaisesRegex(store.SubmissionError, "changed during diagnostics"):
            store.diagnose(cli, SECRETS)

    def test_diagnostics_run_without_a_bundle_and_write_a_safe_job_summary(self):
        with tempfile.TemporaryDirectory() as directory:
            summary = Path(directory) / "summary.md"
            cli = FakeCli()
            with patch.dict(os.environ, dict(SECRETS, GITHUB_STEP_SUMMARY=str(summary))), \
                    patch.object(store.sys, "argv", ["submit-store.py", "--diagnose"]), \
                    patch.object(store, "StoreCli", return_value=cli), \
                    patch.object(store, "submit") as submit, contextlib.redirect_stdout(io.StringIO()):
                store.main()
            self.assertIn("Pending submission ID: none", summary.read_text())
            submit.assert_not_called()

    def test_unexpected_status_does_not_expose_response_content(self):
        for status in (None, {}, "https://storage.example/?sig=hidden", "Published\n::error::hidden"):
            with self.subTest(status=status):
                cli = FakeCli()
                cli.published["status"] = status
                report = store.diagnose(cli, SECRETS)
                self.assertIn("Published submission status: Unknown", report)
                self.assertNotIn("hidden", report)

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
