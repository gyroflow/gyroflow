#!/usr/bin/env python3
"""
Unit Test Suite for Gyroflow Camera & Lens Database Architecture
Issue gyroflow/gyroflow#742: "Refactor lens profile handling"
Maintainer: Adrian Eddy (@AdrianEddy)

Validates:
1. JSON Schema integrity (brand, model, lens mount, sensor size, crop factor, lens metadata)
2. Optical geometry consistency (sensor diagonal vs crop factor physics)
3. Backwards compatibility with legacy .gyroflow project files
4. Fuzzy string resolution and alias matching for real-world user submissions
5. Graceful fallback for unlisted / custom gear preserving all optical parameters
6. Cascading selector filtering for UI comboboxes
"""

import sys
import os
import json
import math
import unittest

# Ensure UTF-8 console output
if sys.stdout and hasattr(sys.stdout, 'reconfigure'):
    sys.stdout.reconfigure(encoding='utf-8')

# Add project root and scripts directory to sys.path
PROJECT_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
if PROJECT_ROOT not in sys.path:
    sys.path.insert(0, PROJECT_ROOT)

from scripts.build_lens_database import (
    LensDatabaseBuilder,
    LensProfileResolver,
    validate_database_schema,
    calculate_crop_factor,
    FULL_FRAME_DIAG
)

DB_PATH = os.path.join(PROJECT_ROOT, "resources", "camera_lens_database.json")


class TestCameraLensDatabaseSchema(unittest.TestCase):
    """Verifies schema definitions, required fields, and data types."""

    @classmethod
    def setUpClass(cls):
        """Ensures database exists or builds it prior to running tests."""
        if not os.path.exists(DB_PATH):
            builder = LensDatabaseBuilder()
            builder.load_curated_data()
            builder.parse_lensfun_xmls()
            builder.parse_gyroflow_presets()
            builder.export(DB_PATH)

        with open(DB_PATH, "r", encoding="utf-8") as f:
            cls.db = json.load(f)

    def test_database_structure(self):
        """Verifies top-level keys and schema version."""
        self.assertIn("schema_version", self.db)
        self.assertEqual(self.db["schema_version"], "2.0.0")
        self.assertIn("cameras", self.db)
        self.assertIn("lenses", self.db)
        self.assertIn("mounts", self.db)
        self.assertIn("compatibility_index", self.db)
        self.assertIn("stats", self.db)
        self.assertGreater(self.db["stats"]["total_cameras"], 100)
        self.assertGreater(self.db["stats"]["total_lenses"], 500)
        self.assertGreater(self.db["stats"]["total_mounts"], 15)

    def test_schema_validator(self):
        """Runs the built-in comprehensive schema validation routine."""
        is_valid, errors = validate_database_schema(self.db)
        self.assertTrue(is_valid, f"Schema validation failed: {errors[:5]}")
        self.assertEqual(len(errors), 0)

    def test_camera_schema_fields(self):
        """Ensures every camera has brand, model, mount, sensor dimensions, and crop factor."""
        cameras = self.db["cameras"]
        self.assertGreater(len(cameras), 0)

        for cam in cameras:
            c_id = cam.get("id")
            self.assertTrue(bool(c_id), "Camera must have an 'id'")
            self.assertTrue(bool(cam.get("brand")), f"Camera {c_id} missing 'brand'")
            self.assertTrue(bool(cam.get("model")), f"Camera {c_id} missing 'model'")
            self.assertTrue(bool(cam.get("lens_mount")), f"Camera {c_id} missing 'lens_mount'")
            self.assertIsInstance(cam.get("aliases", []), list, f"Camera {c_id} aliases must be a list")

            # Validate sensor size
            ss = cam.get("sensor_size", {})
            self.assertIn("width", ss, f"Camera {c_id} missing sensor width")
            self.assertIn("height", ss, f"Camera {c_id} missing sensor height")
            self.assertGreater(ss["width"], 0.0, f"Camera {c_id} sensor width must be > 0")
            self.assertGreater(ss["height"], 0.0, f"Camera {c_id} sensor height must be > 0")

            # Validate crop factor
            cf = cam.get("crop_factor")
            self.assertIsNotNone(cf, f"Camera {c_id} missing crop factor")
            self.assertGreater(cf, 0.2, f"Camera {c_id} crop factor must be > 0.2")
            self.assertLess(cf, 20.0, f"Camera {c_id} crop factor must be < 20.0")

    def test_lens_schema_fields(self):
        """Ensures every lens has brand, model, mounts, focal range, and aperture."""
        lenses = self.db["lenses"]
        self.assertGreater(len(lenses), 0)

        for lens in lenses:
            l_id = lens.get("id")
            self.assertTrue(bool(l_id), "Lens must have an 'id'")
            self.assertTrue(bool(lens.get("brand")), f"Lens {l_id} missing 'brand'")
            self.assertTrue(bool(lens.get("model")), f"Lens {l_id} missing 'model'")

            # Validate mount compatibility
            mounts = lens.get("mount_compatibility", [])
            self.assertIsInstance(mounts, list, f"Lens {l_id} mounts must be a list")
            self.assertGreater(len(mounts), 0, f"Lens {l_id} must have >= 1 mount")

            # Validate focal range
            fr = lens.get("focal_range", {})
            self.assertIn("min", fr, f"Lens {l_id} missing min focal length")
            self.assertIn("max", fr, f"Lens {l_id} missing max focal length")
            self.assertGreater(fr["min"], 0.0, f"Lens {l_id} min focal must be > 0")
            self.assertGreaterEqual(fr["max"], fr["min"], f"Lens {l_id} max focal must be >= min focal")

            # Validate aperture
            ap = lens.get("aperture", {})
            self.assertIn("min_f_stop", ap, f"Lens {l_id} missing min_f_stop")
            self.assertIn("max_f_stop", ap, f"Lens {l_id} missing max_f_stop")
            self.assertGreater(ap["min_f_stop"], 0.5, f"Lens {l_id} min_f_stop must be > 0.5")


class TestOpticalGeometryAndPhysics(unittest.TestCase):
    """Verifies that sensor dimensions and crop factor physics are mathematically consistent."""

    @classmethod
    def setUpClass(cls):
        with open(DB_PATH, "r", encoding="utf-8") as f:
            cls.db = json.load(f)
        cls.cameras_by_id = {c["id"]: c for c in cls.db["cameras"]}

    def test_full_frame_cameras_crop_factor(self):
        """Full frame cameras must have crop factor ~1.0."""
        ff_models = [
            "sony_alpha_7_iv", "sony_fx3", "canon_eos_r5",
            "nikon_z8", "panasonic_lumix_s5_ii", "blackmagic_cinema_camera_6k"
        ]
        for c_id in ff_models:
            if c_id in self.cameras_by_id:
                cam = self.cameras_by_id[c_id]
                self.assertAlmostEqual(cam["crop_factor"], 1.0, delta=0.08, msg=f"Camera {c_id} crop factor should be ~1.0")
                self.assertGreaterEqual(cam["sensor_size"]["width"], 35.0)
                self.assertGreaterEqual(cam["sensor_size"]["height"], 23.0)

    def test_aps_c_cameras_crop_factor(self):
        """APS-C cameras must have crop factor between 1.5 and 1.65."""
        apsc_models = ["sony_fx30", "sony_a6700", "fujifilm_x_t4", "canon_eos_r7"]
        for c_id in apsc_models:
            if c_id in self.cameras_by_id:
                cam = self.cameras_by_id[c_id]
                self.assertTrue(1.50 <= cam["crop_factor"] <= 1.65, f"Camera {c_id} crop factor {cam['crop_factor']} not in APS-C range")

    def test_micro_four_thirds_crop_factor(self):
        """Micro Four Thirds cameras must have crop factor between 1.95 and 2.10."""
        mft_models = ["blackmagic_bmpcc_4k", "panasonic_lumix_gh6"]
        for c_id in mft_models:
            if c_id in self.cameras_by_id:
                cam = self.cameras_by_id[c_id]
                self.assertTrue(1.95 <= cam["crop_factor"] <= 2.10, f"Camera {c_id} crop factor {cam['crop_factor']} not in MFT range")

    def test_action_cams_sensor_dimensions(self):
        """Verifies sensor sizes for 1/1.9-inch (GoPro) and 1/1.3-inch (DJI Action 4)."""
        if "gopro_hero11_black" in self.cameras_by_id:
            hero11 = self.cameras_by_id["gopro_hero11_black"]
            self.assertAlmostEqual(hero11["sensor_size"]["width"], 6.40, delta=0.2)
            self.assertAlmostEqual(hero11["sensor_size"]["height"], 5.60, delta=0.2)

        if "dji_osmo_action_4" in self.cameras_by_id:
            action4 = self.cameras_by_id["dji_osmo_action_4"]
            self.assertAlmostEqual(action4["sensor_size"]["width"], 9.60, delta=0.3)
            self.assertAlmostEqual(action4["sensor_size"]["height"], 7.20, delta=0.3)

    def test_optical_diagonal_formula(self):
        """Tests the mathematical helper calculate_crop_factor."""
        cf_35mm = calculate_crop_factor(36.0, 24.0)
        self.assertAlmostEqual(cf_35mm, 1.0, places=2)

        cf_apsc = calculate_crop_factor(23.5, 15.6)
        self.assertAlmostEqual(cf_apsc, 1.534, delta=0.02)

        cf_mft = calculate_crop_factor(17.3, 13.0)
        self.assertAlmostEqual(cf_mft, 2.0, delta=0.03)


class TestBackwardsCompatibilityAndResolution(unittest.TestCase):
    """
    Verifies that legacy .gyroflow project files with messy, unstructured
    strings resolve reliably into canonical hardware entities.
    """

    @classmethod
    def setUpClass(cls):
        cls.resolver = LensProfileResolver(DB_PATH)

    def test_resolve_sony_variations(self):
        """Resolves various user variants for Sony A7 IV and Sigma 24-70."""
        variants = [
            ("Sony", "a7iv", "Sigma 24-70mm f2.8 DG DN"),
            ("SONY", "A7 IV", "24-70mm F2.8 DG DN Art"),
            ("sony", "ILCE-7M4", "Sigma 24-70mm f2.8"),
            ("Sony", "A74", "Sigma 24-70 2.8"),
            ("Sony", "Alpha 7M4", "Sigma 24-70mm")
        ]
        for b, m, l in variants:
            cam = self.resolver.resolve_camera(b, m)
            self.assertIsNotNone(cam, f"Failed to resolve camera: {b} {m}")
            self.assertEqual(cam["id"], "sony_alpha_7_iv")
            self.assertEqual(cam["brand"], "Sony")
            self.assertEqual(cam["model"], "Alpha 7 IV")
            self.assertEqual(cam["lens_mount"], "Sony E")

            lens = self.resolver.resolve_lens(l, cam)
            self.assertIsNotNone(lens, f"Failed to resolve lens: {l}")
            self.assertIn("sigma", lens["id"].lower())
            self.assertIn("24_70", lens["id"].lower())

    def test_resolve_gopro_variations(self):
        """Resolves GoPro models and handles non-interchangeable built-in lens."""
        gopro_variants = [
            ("GoPro", "Hero 10", "Built-in"),
            ("GOPRO", "HERO10 Black", ""),
            ("gopro", "Hero10", None),
            ("GoPro", "HERO 10 Black", "Wide"),
            ("GoPro", "HERO11 Black", "Wide"),
            ("GoPro", "Hero 12", "HyperView"),
        ]
        for b, m, l in gopro_variants:
            cam = self.resolver.resolve_camera(b, m)
            self.assertIsNotNone(cam, f"Failed to resolve GoPro: {b} {m}")
            self.assertEqual(cam["brand"], "GoPro")
            self.assertIn("hero", cam["model"].lower())
            self.assertEqual(cam["lens_mount"], "GoPro Fixed")
            self.assertFalse(cam["is_interchangeable_lens"])

            lens = self.resolver.resolve_lens(l, cam)
            self.assertIsNotNone(lens, f"GoPro should provide built-in lens for {m}")

    def test_resolve_blackmagic_and_red_cinema(self):
        """Resolves BMPCC 4K/6K and RED Komodo variants."""
        cinema_variants = [
            ("Blackmagic", "BMPCC 6k", "blackmagic_bmpcc_6k"),
            ("Blackmagic Design", "Pocket Cinema Camera 6K", "blackmagic_bmpcc_6k"),
            ("Blackmagic", "Pocket Cinema Camera 4k", "blackmagic_bmpcc_4k"),
            ("RED", "KOMODO 6K", "red_komodo_6k"),
            ("RED Digital Cinema", "komodo", "red_komodo_6k"),
            ("RED", "Komodo X", "red_komodo_x_6k"),
            ("RED", "V-RAPTOR 8K VV", "red_v_raptor_8k_vv"),
        ]
        for b, m, expected_id in cinema_variants:
            cam = self.resolver.resolve_camera(b, m)
            self.assertIsNotNone(cam, f"Failed to resolve cinema camera: {b} {m}")
            self.assertEqual(cam["id"], expected_id)

    def test_resolve_dji_action_and_drone(self):
        """Resolves DJI Action 4, Mini 3 Pro, and O3 Air Unit."""
        dji_variants = [
            ("DJI", "Action 4", "dji_osmo_action_4"),
            ("DJI", "Osmo Action 4", "dji_osmo_action_4"),
            ("DJI", "Action 3", "dji_osmo_action_3"),
            ("DJI", "Mini 3 Pro", "dji_mini_3_pro"),
            ("DJI", "MINI 3 PRO", "dji_mini_3_pro"),
            ("DJI", "O3 Air Unit", "dji_o3_air_unit"),
            ("DJI", "Avata 2", "dji_avata_2"),
        ]
        for b, m, expected_id in dji_variants:
            cam = self.resolver.resolve_camera(b, m)
            self.assertIsNotNone(cam, f"Failed to resolve DJI device: {b} {m}")
            self.assertEqual(cam["id"], expected_id)

    def test_legacy_gyroflow_project_roundtrip(self):
        """
        Loads a mock legacy .gyroflow project file, tests resolution,
        and verifies that NO optical data or stabilization coefficients are dropped.
        """
        legacy_project = {
            "gyroflow_version": "1.5.4",
            "camera_brand": "SONY",
            "camera_model": "a7iv",
            "lens_model": "Sigma 24-70mm f2.8 DG DN",
            "video_info": {
                "width": 3840,
                "height": 2160,
                "fps": 23.976,
                "duration_ms": 14500
            },
            "lens_profile": {
                "calibrated_by": "Adrian Eddy",
                "calib_dimension": {"w": 3840, "h": 2160},
                "distortion_model": "fisheye",
                "coefficients": [0.052, -0.012, 0.003, 0.0]
            },
            "stabilization": {
                "smoothness": 0.5,
                "fov": 1.0,
                "roll_horizon": True
            }
        }

        resolved = self.resolver.resolve_legacy_project(legacy_project)

        # Verify preservation of original project metadata
        self.assertEqual(resolved["gyroflow_version"], "1.5.4")
        self.assertEqual(resolved["video_info"]["width"], 3840)
        self.assertEqual(resolved["stabilization"]["smoothness"], 0.5)
        self.assertEqual(resolved["lens_profile"]["distortion_model"], "fisheye")

        # Verify resolved canonical camera
        self.assertIn("canonical_camera", resolved)
        cc = resolved["canonical_camera"]
        self.assertEqual(cc["id"], "sony_alpha_7_iv")
        self.assertEqual(cc["brand"], "Sony")
        self.assertEqual(cc["model"], "Alpha 7 IV")
        self.assertEqual(cc["lens_mount"], "Sony E")
        self.assertEqual(cc["crop_factor"], 1.0)

        # Verify resolved canonical lens
        self.assertIn("canonical_lens", resolved)
        cl = resolved["canonical_lens"]
        self.assertIn("sigma", cl["id"])
        self.assertEqual(cl["focal_range"]["min"], 24.0)
        self.assertEqual(cl["focal_range"]["max"], 70.0)

    def test_custom_camera_graceful_fallback(self):
        """
        Verifies that completely unlisted custom cameras safely fall back
        to 'Other / Custom' without crashing or causing data loss.
        """
        custom_project = {
            "gyroflow_version": "1.5.4",
            "camera_brand": "ExperimentalLab",
            "camera_model": "Prototype-X1",
            "lens_model": "PinHole-99",
            "video_info": {"width": 1920, "height": 1080}
        }

        resolved = self.resolver.resolve_legacy_project(custom_project)

        self.assertIn("canonical_camera", resolved)
        self.assertTrue(resolved["canonical_camera"].get("is_custom", False))
        self.assertEqual(resolved["canonical_camera"]["brand"], "ExperimentalLab")
        self.assertEqual(resolved["canonical_camera"]["model"], "Prototype-X1")

        self.assertIn("canonical_lens", resolved)
        self.assertTrue(resolved["canonical_lens"].get("is_custom", False))
        self.assertEqual(resolved["canonical_lens"]["model"], "PinHole-99")


class TestCascadingSelectorsAndUIIntegration(unittest.TestCase):
    """
    Verifies data structures necessary for Calibrator cascading dropdowns
    (Brand -> Camera Model -> Lens Model).
    """

    @classmethod
    def setUpClass(cls):
        with open(DB_PATH, "r", encoding="utf-8") as f:
            cls.db = json.load(f)

    def test_brand_combobox_data(self):
        """Brand list must contain all primary manufacturers."""
        brands = self.db.get("camera_brands", [])
        expected_brands = ["Sony", "GoPro", "DJI", "Blackmagic Design", "RED Digital Cinema", "Canon", "Fujifilm", "Nikon", "Panasonic"]
        for eb in expected_brands:
            self.assertIn(eb, brands, f"Brand {eb} should be available in brand list")

    def test_camera_combobox_filtering(self):
        """Filtering by brand must only return cameras belonging to that brand."""
        brand_map = self.db["compatibility_index"]["brand_to_cameras"]
        self.assertIn("Sony", brand_map)
        sony_cams = brand_map["Sony"]
        self.assertGreater(len(sony_cams), 15)

        for cam in sony_cams:
            self.assertTrue(bool(cam["model"]))
            self.assertTrue(bool(cam["lens_mount"]))
            self.assertGreater(cam["crop_factor"], 0.5)

    def test_lens_combobox_mount_filtering(self):
        """Lenses must be filterable by lens mount (e.g. Sony E)."""
        mount_map = self.db["compatibility_index"]["mount_to_lenses"]
        self.assertIn("Sony E", mount_map)
        sony_e_lenses = mount_map["Sony E"]
        self.assertGreater(len(sony_e_lenses), 20)

        for lens in sony_e_lenses:
            self.assertTrue(bool(lens["model"]))
            self.assertGreater(lens["focal_range"]["min"], 0.0)


if __name__ == "__main__":
    unittest.main()
