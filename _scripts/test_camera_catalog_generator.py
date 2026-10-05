"""One end-to-end generator regression using minimal input fixtures."""
import json
from pathlib import Path
import tempfile
import unittest
from generate_camera_catalog import generate

class CatalogueGeneration(unittest.TestCase):
    def test_deterministic_merged_metadata_preserves_unknowns(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            profiles, lensfun = root / "profiles", root / "lensfun"
            profiles.mkdir(); lensfun.mkdir()
            (profiles / "sample.json").write_text(json.dumps({"camera_brand":"Example","camera_model":"Unknown body","lens_model":"Unbranded lens","calib_dimension":{"w":1920,"h":1080}}))
            (lensfun / "example.xml").write_text('''<lensdatabase version="2"><camera><maker>Sony</maker><model>ILCE-7M4</model><model lang="en">Alpha 7 IV</model><mount>Sony E</mount><cropfactor>1</cropfactor></camera><lens><maker>Sony</maker><model>Power Zoom</model><mount>Sony E</mount><cropfactor>1</cropfactor><calibration><distortion model="poly3" focal="24"/><distortion model="poly3" focal="70"/></calibration></lens></lensdatabase>''')
            (profiles / "alias.json").write_text(json.dumps({"camera_brand":"Sony","camera_model":"Alpha 7 IV","calib_dimension":{"w":1920,"h":1080}}))
            first = generate(profiles, lensfun, "fixture-profiles", "fixture-lensfun")
            sony = [c for c in first["cameras"] if c["brand"] == "Sony"]
            self.assertEqual(len(sony), 1)
            self.assertEqual(sony[0]["model"], "ILCE-7M4")
            self.assertIn("Alpha 7 IV", sony[0]["aliases"])
            self.assertEqual(first, generate(profiles, lensfun, "fixture-profiles", "fixture-lensfun"))
            unknown = next(c for c in first["cameras"] if c["brand"] == "Example")
            self.assertEqual(unknown["mounts"], [])
            self.assertNotIn("crop_factor", unknown)
            self.assertEqual(first["lenses"][0]["sampled_focal_lengths"], [24.0,70.0])
            self.assertNotIn("profiles", first)
            (lensfun / "example.xml").unlink()
            with self.assertRaises(ValueError): generate(profiles,lensfun,"fixture-profiles","fixture-lensfun")

if __name__ == "__main__": unittest.main()
