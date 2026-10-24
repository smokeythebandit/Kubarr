"""VPN preflight must validate behavior, including shared chart helpers."""
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
CHARTS = Path(os.environ.get("KUBARR_ACCEPTANCE_CHARTS_DIR", ROOT.parent / "kubarr-charts"))
LAB = ROOT / "tests/acceptance/vpn/lab.sh"


class RenderedExtraEnvTests(unittest.TestCase):
    def check(self, chart):
        return subprocess.run([
            "bash", "-euc",
            'source "$1"; fail() { printf "%s\\n" "$*" >&2; return 1; }; vpn_lab_assert_extra_env "$2"',
            "fixture", str(LAB), str(chart),
        ], capture_output=True, text=True)

    def test_shared_helper_renders_extra_env_without_mutating_checkout(self):
        chart = CHARTS / "media-manager/sonarr"
        before = (chart / "templates/deployment.yaml").read_bytes()
        result = self.check(chart)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((chart / "templates/deployment.yaml").read_bytes(), before)

    def test_values_field_without_a_rendered_sidecar_is_rejected(self):
        with tempfile.TemporaryDirectory(prefix="kubarr-vpn-preflight-") as directory:
            root = Path(directory)
            chart = root / "media-manager/sonarr"
            shutil.copytree(CHARTS / "media-manager/sonarr", chart)
            shutil.copytree(CHARTS / "system/kubarr-common", root / "system/kubarr-common")
            template = chart / "templates/deployment.yaml"
            template.write_text(re.sub(
                r'^.*include "kubarr-common.gluetun.container".*\n', "", template.read_text(), flags=re.M,
            ))
            result = self.check(chart)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("must render vpn.extraEnv exactly once", result.stderr)
