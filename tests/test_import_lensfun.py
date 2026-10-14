import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


class ImportLensfunTests(unittest.TestCase):
    def test_import_lensfun_generates_ptlens_profile(self):
        fixture = Path(__file__).parent / "fixtures" / "lensfun_sample.xml"
        with tempfile.TemporaryDirectory() as tmp:
            output_dir = Path(tmp) / "profiles"
            result = subprocess.run(
                [sys.executable, "tools/import_lensfun.py", str(fixture), str(output_dir)],
                check=True,
                capture_output=True,
                text=True,
            )

            self.assertIn("Generated 1 Gyroflow Lensfun profile", result.stdout)
            profiles = list(output_dir.glob("*.json"))
            self.assertEqual(len(profiles), 1)

            profile = json.loads(profiles[0].read_text())
            self.assertEqual(profile["camera_brand"], "Sony")
            self.assertEqual(profile["camera_model"], "A7S III")
            self.assertEqual(profile["lens_model"], "Tamron 28-75mm f/2.8 Di III RXD")
            self.assertEqual(profile["distortion_model"], "ptlens")
            self.assertEqual(profile["fisheye_params"]["distortion_coeffs"], [0.012, -0.034, 0.005])
            self.assertEqual(profile["crop_factor"], 1.0)

    def test_import_lensfun_skips_unsupported_distortion_models(self):
        fixture = Path(__file__).parent / "fixtures" / "lensfun_sample.xml"
        with tempfile.TemporaryDirectory() as tmp:
            output_dir = Path(tmp) / "profiles"
            subprocess.run(
                [sys.executable, "tools/import_lensfun.py", str(fixture), str(output_dir)],
                check=True,
            )

            text = "\n".join(path.read_text() for path in output_dir.glob("*.json"))
            self.assertNotIn("Unsupported Model", text)


if __name__ == "__main__":
    unittest.main()
