"""Offline contracts for the disposable Helm stack; no cluster is contacted."""

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).parent
spec = importlib.util.spec_from_file_location("stack_fixture", ROOT / "stack-fixture.py")
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)

CHARTS = (
    "system/managed-nfs", "system/postgresql", "monitoring/fluent-bit",
    "monitoring/victorialogs", "monitoring/victoriametrics",
    "system/kubarr-backend", "system/kubarr-frontend", "system/openresty",
    "system/kubarr-worker",
)
MOCK_TOOL = r'''#!/usr/bin/env python3
import json, os, pathlib, sys
name, args = pathlib.Path(sys.argv[0]).name, sys.argv[1:]
body = sys.stdin.read() if ("apply" in args or "-i" in args) else ""
with open(os.environ["CALLS"], "a") as log:
    log.write(json.dumps({"tool": name, "args": args, "stdin": body}) + "\n")
if name == "kubectl" and args == ["config", "current-context"]:
    print(os.environ["FAKE_CONTEXT"])
elif name == "kubectl" and "-tAc" in args:
    print("t")
elif name == "helm" and args[:2] == ["upgrade", "--install"]:
    if args[3] == os.environ.get("FAIL_RELEASE"):
        sys.exit(42)
'''


class StackDeployTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        for tool in ("helm", "kubectl"):
            path = self.bin / tool
            path.write_text(MOCK_TOOL)
            path.chmod(0o755)
        self.charts = self.root / "charts"
        for chart in CHARTS:
            path = self.charts / chart
            path.mkdir(parents=True)
            (path / "Chart.yaml").write_text("name: fixture\nversion: 1.0.0\n")
        self.log = self.root / "calls.jsonl"
        self.env = {
            **os.environ,
            "PATH": f"{self.bin}:{os.environ['PATH']}",
            "KUBECONFIG": "/dev/null",
            "TEST_CONTEXT": "kind-kubarr-acceptance-fixture",
            "FAKE_CONTEXT": "kind-kubarr-acceptance-fixture",
            "TEST_CHARTS_DIR": str(self.charts),
            "TEST_IMAGE_TAG": "ci-fixture",
            "TEST_STORAGE_SIZE": "2Gi",
            "TEST_USERNAME": "test-admin",
            "TEST_PASSWORD": "synthetic-'password",
            "TEST_ADMIN_EMAIL": "admin@example.invalid",
            "CALLS": str(self.log),
        }

    def deploy(self, **overrides):
        result = subprocess.run(
            ["bash", str(ROOT / "deploy-stack.sh")],
            env={**self.env, **overrides}, capture_output=True, text=True, timeout=10,
        )
        calls = [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []
        return result, calls

    def test_deploys_pinned_charts_local_images_and_private_gateway_then_seeds(self):
        result, calls = self.deploy()
        self.assertEqual(result.returncode, 0, result.stderr)
        installs = [c for c in calls if c["tool"] == "helm" and c["args"][:2] == ["upgrade", "--install"]]
        self.assertEqual({c["args"][3] for c in installs}, {Path(c).name for c in CHARTS})
        self.assertEqual(len(installs), 9)
        for call in installs:
            args = call["args"]
            self.assertEqual(args[args.index("--kube-context") + 1], self.env["TEST_CONTEXT"])
            self.assertEqual(args[args.index("-n") + 1], "default")
            self.assertIn("--server-side=false", args)
            self.assertIn("--wait=legacy", args)
            if args[3] in ("kubarr-backend", "kubarr-frontend", "kubarr-worker"):
                self.assertIn(f"image.repository={args[3]}", args)
                self.assertIn("image.tag=ci-fixture", args)
                self.assertIn("image.pullPolicy=Never", args)
            if args[3] in ("openresty", "kubarr-backend"):
                self.assertIn("service.type=ClusterIP", args)
            self.assertTrue(args[4].startswith(str(self.charts)))
        for call in calls:
            self.assertNotIn(self.env["TEST_PASSWORD"], " ".join(call["args"]))
            if call["tool"] == "kubectl" and call["args"][0] != "config":
                self.assertEqual(call["args"][:2], ["--context", self.env["TEST_CONTEXT"]])
        self.assertIn("CREATE EXTENSION IF NOT EXISTS pgcrypto", calls[-1]["stdin"])
        self.assertIn("synthetic-''password", calls[-1]["stdin"])
        secret = json.loads(next(c["stdin"] for c in calls if "apply" in c["args"]))
        self.assertEqual(secret["metadata"]["namespace"], "kubarr-backend")
        self.assertEqual(json.loads(secret["stringData"]["config.json"])["config_json"]["size"], "2Gi")
        self.assertNotIn(self.env["TEST_PASSWORD"], result.stdout + result.stderr)

    def test_rejects_other_cluster_before_changes(self):
        result, calls = self.deploy(FAKE_CONTEXT="kind-kubarr-local")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0]["args"], ["config", "current-context"])

    def test_rejects_non_test_context_without_contacting_cluster(self):
        result, calls = self.deploy(TEST_CONTEXT="kind-kubarr-local")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, [])

    def test_missing_chart_prevents_mutations(self):
        (self.charts / "system/kubarr-worker/Chart.yaml").unlink()
        result, calls = self.deploy()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(calls), 1)

    def test_helm_failure_stops_before_database_seed(self):
        result, calls = self.deploy(FAIL_RELEASE="kubarr-backend")
        self.assertEqual(result.returncode, 42)
        self.assertFalse(any(c["stdin"] for c in calls))
        self.assertEqual(calls[-1]["args"][3], "kubarr-backend")

    def test_seed_escapes_data_and_requires_an_admin_role(self):
        sql = fixture.seed_sql("o'brien", "p'ass\\word", "a'b@example.invalid", "5Gi")
        self.assertIn("'o''brien'", sql)
        self.assertIn("crypt('p''ass\\word', gen_salt('bf', 12))", sql)
        self.assertIn("RAISE EXCEPTION 'Admin role migrations are not ready'", sql)
        self.assertTrue(sql.startswith("BEGIN;"))
        self.assertTrue(sql.rstrip().endswith("COMMIT;"))


if __name__ == "__main__":
    unittest.main()
