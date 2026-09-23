import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


REDACTOR = Path(__file__).with_name("redact_output.py")
CONFIG_ERROR = "browser output redaction configuration failed"


class RedactOutputTests(unittest.TestCase):
    def run_redactor(self, input_text, credentials_path, password):
        env = os.environ.copy()
        env["VPN_LAB_CREDENTIALS_FILE"] = str(credentials_path)
        env["ADMIN_PASSWORD"] = password
        return subprocess.run(
            [sys.executable, str(REDACTOR)],
            input=input_text,
            text=True,
            capture_output=True,
            env=env,
            check=False,
        )

    def test_redacts_plaintext_and_json_escaped_credentials(self):
        private_key = 'fake/private+key="value"'
        password = 'fake-admin-"password"\\suffix'
        with tempfile.TemporaryDirectory() as directory:
            credentials_path = Path(directory) / "credentials.json"
            credentials_path.write_text(
                json.dumps({"private_key": private_key, "provider_name": "test"}),
                encoding="utf-8",
            )
            input_text = "\n".join(
                (
                    "normal browser diagnostic",
                    f"fill({password})",
                    f"private key: {private_key}",
                    json.dumps({"password": password, "private_key": private_key}),
                )
            )
            result = self.run_redactor(input_text, credentials_path, password)

        self.assertEqual(result.returncode, 0)
        self.assertIn("normal browser diagnostic", result.stdout)
        self.assertIn("[REDACTED]", result.stdout)
        for secret in (
            password,
            private_key,
            json.dumps(password)[1:-1],
            json.dumps(private_key)[1:-1],
        ):
            self.assertNotIn(secret, result.stdout)
            self.assertNotIn(secret, result.stderr)

    def test_missing_credentials_file_fails_closed(self):
        secret = "raw-output-secret"
        with tempfile.TemporaryDirectory() as directory:
            result = self.run_redactor(
                f"diagnostic containing {secret}\n",
                Path(directory) / "missing.json",
                "admin-secret",
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "")
        self.assertIn(CONFIG_ERROR, result.stderr)
        self.assertNotIn(secret, result.stdout + result.stderr)
        self.assertNotIn("missing.json", result.stdout + result.stderr)

    def test_malformed_credentials_file_fails_closed(self):
        secret = "raw-malformed-secret"
        with tempfile.TemporaryDirectory() as directory:
            credentials_path = Path(directory) / "credentials.json"
            credentials_path.write_text(
                '["not-json-private-data"]', encoding="utf-8"
            )
            result = self.run_redactor(
                f"diagnostic containing {secret}\n", credentials_path, "admin-secret"
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "")
        self.assertIn(CONFIG_ERROR, result.stderr)
        self.assertNotIn(secret, result.stdout + result.stderr)
        self.assertNotIn("not-json-private-data", result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
