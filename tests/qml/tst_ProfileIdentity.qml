import QtQuick
import QtTest
import "../../src/ui/menu/CameraIdentity.js" as CameraIdentity
import "../../src/ui/menu/ProfileReview.js" as ProfileReview
import "../../src/ui/menu/ProfileSubmission.js" as ProfileSubmission

TestCase {
    name: "ProfileIdentity"
    when: true

    function test_camera_identity_and_catalog_requirement() {
        compare(CameraIdentity.selectorModel("Any", ["Sony", "Sony", ""], "Other"), ["Any", "Sony", "Other"]);
        compare(CameraIdentity.selectorValue(2, ["Any", "Sony", "Other"], true), "");
        compare(CameraIdentity.cameraNeedsLens({"fixed_lens": true}), false);
        compare(CameraIdentity.cameraNeedsLens({}), true);
        compare(CameraIdentity.detectedCamera({"brand": "Sony", "lens_model": "FE", "lens_info": "Zoom"}).lens, "FE");
    }

    function test_review_identity_matching() {
        const profiles = [["First", "first.json", "one"], ["Selected", "selected.json", "selected"]];
        compare(ProfileReview.findIndex(profiles, "", "selected"), 1);
        compare(ProfileReview.findExactIndex(profiles, "selected.json", "selected"), 1);
        compare(ProfileReview.findIndex(profiles, "missing.json", "missing"), -1);
    }

    function test_upload_validation_and_snapshot() {
        const metadata = {"camera_brand":"Sony", "camera_model":"A7", "lens_model":"24-70mm", "focal_length":35};
        const selection = {"brandIndex":1, "brandValue":"Sony", "modelIndex":0, "modelValue":"", "lensIndex":0, "lensValue":"", "hasFocalLength":true};
        compare(ProfileSubmission.validationErrors(metadata, selection, true), ["lens_name_required"]);
        metadata.lens_model = "Sony FE 24-70mm f/2.8";
        compare(ProfileSubmission.validationErrors(metadata, selection, true), []);
        metadata.lens_model = "24-70mm";
        selection.hasFocalLength = false;
        compare(ProfileSubmission.validationErrors(metadata, selection, true), ["lens_name_required", "zoom_focal_length_required"]);
        const snapshot = ProfileSubmission.metadataSnapshot(metadata);
        snapshot.camera_brand = "Canon";
        compare(metadata.camera_brand, "Sony");
    }
}
