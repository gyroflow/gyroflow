// SPDX-License-Identifier: GPL-3.0-or-later
// Run: qmltestrunner -input tests/qml -platform offscreen
import QtQuick
import QtTest
import "../../src/ui/components"

Item {
    id: scene;
    width: 360;
    height: 900;
    property real dpiScale: 1;
    property bool isMobile: false;
    property string style: "dark";
    property string styleFont: "Arial";
    property color styleTextColor: "white";
    property color styleTextColorOnAccent: "white";
    property color styleButtonColor: "#333333";
    property color styleAccentColor: "#3388ee";
    property color stylePopupBorder: "#555555";
    property color styleHighlightColor: "#444444";
    property color styleBackground: "#222222";
    property color styleBackground2: "#222222";
    property var window: scene;
    property var main_window: scene;
    property var settings: settingsObject;
    property var controller: controllerObject;

    QtObject {
        id: settingsObject;
        property var values: ({});
        function value(key, fallback) { return values[key] === undefined ? fallback : values[key]; }
        function setValue(key, value) { values[key] = value; }
    }
    QtObject {
        id: controllerObject;
        signal all_profiles_loaded();
        property string loaded: "";
        property bool largeCatalogue: false;
        function camera_selector_options(text) {
            let s = JSON.parse(text);
            if (s.camera_model === "a6000") s.camera_model = "ILCE-6000";
            return JSON.stringify({selection:s, brands:largeCatalogue ? Array.from({length:250}, (_,i) => "Brand " + i) : ["Sony", "Other maker"], models:s.camera_brand === "Sony" ? ["ILCE-6000", "ILCE-6300"] : [],
                model_aliases:{"ILCE-6000":["Alpha 6000","a6000"]},
                lenses:s.camera_model ? [{name:"Sony E 16-50mm", zoom:true, adapted:false, has_profiles:true}] : [],
                camera:s.camera_model === "ILCE-6000" ? {mounts:["Sony E"], crop_factor:1.5} : null});
        }
        function browse_lens_profiles(text, compatible) {
            const s = JSON.parse(text);
            if (s.camera_brand !== "Sony") return "[]";
            return JSON.stringify([1,2,3].map(i => ({id:"profile"+i, checksum:"crc"+i, name:"Calibration "+i,
                width:1920,height:1080,fps:30,suggested:i===3,official:false,rating:0})).filter(p => compatible || !p.suggested));
        }
        function load_lens_profile(id) { loaded = id; }
    }
    QtObject {
        id: menu;
        property string profileChecksum: "";
        property int currentVideoAspectRatio: 0;
        property int currentVideoAspectRatioSwapped: 0;
        property bool selected_manually: false;
        property var favorites: ({});
        function updateFavorites() {}
        function loadFavorites() {}
    }

    Component { id: selectorComponent; CameraSelector { allowOther: true; } }
    Component { id: browserComponent; ProfileBrowser { profilesMenu: menu; } }

    TestCase {
        name: "CameraAndProfileSelection";
        when: windowShown;
        function init() { controllerObject.largeCatalogue = false; settingsObject.values = {}; controllerObject.loaded = ""; menu.profileChecksum = ""; }

        function test_large_catalogue_has_a_scrollable_viewport() {
            controllerObject.largeCatalogue = true;
            const selector = createTemporaryObject(selectorComponent, scene);
            const brand = findChild(selector, "cameraBrandSelector");
            brand.popup.open();
            tryCompare(brand.popup, "visible", true);
            verify(brand.popup.height <= 280);
            verify(brand.popup.lv.contentHeight > brand.popup.height);
            brand.popup.close();
        }

        function test_filter_preserves_selection_until_a_match_is_chosen() {
            const selector = createTemporaryObject(selectorComponent, scene);
            selector.setSelection("Sony", "ILCE-6000", "Sony E 16-50mm");
            const brand = findChild(selector, "cameraBrandSelector");
            brand.popup.open();
            const filter = findChild(brand.popup, "catalogueFilter");
            tryCompare(filter, "activeFocus", true);
            filter.text = "other maker";
            compare(selector.cameraBrand, "Sony");
            compare(brand.filteredOptions[0].text, "Other maker");
            filter.accepted();
            compare(selector.cameraBrand, "Other maker");
            compare(selector.cameraModel, "");
        }

        function test_prefill_survives_catalogue_reload() {
            const selector = createTemporaryObject(selectorComponent, scene);
            verify(selector);
            selector.setSelection("Sony", "a6000", "Sony E 16-50mm");
            compare(selector.cameraModel, "ILCE-6000");
            compare(selector.lensModel, "Sony E 16-50mm");
            const model = findChild(selector, "cameraModelSelector");
            model.query = "a6000";
            compare(model.filteredOptions[0].text, "ILCE-6000");
            model.query = "ILCE 6000";
            compare(model.filteredOptions[0].text, "ILCE-6000");
            verify(selector.zoomLens);
            controllerObject.all_profiles_loaded();
            compare(selector.cameraModel, "ILCE-6000");
            compare(selector.lensModel, "Sony E 16-50mm");
        }

        function test_other_allows_manual_models_and_resets_dependent_fields() {
            const selector = createTemporaryObject(selectorComponent, scene);
            selector.setSelection("Sony", "ILCE-6000", "Sony E 16-50mm");
            const model = findChild(selector, "cameraModelSelector");
            model.currentIndex = model.count - 1;
            model.activated(model.currentIndex);
            verify(selector.manualModel);
            const input = findChild(selector, "cameraModelInput");
            input.text = "Future camera";
            input.editingFinished();
            compare(selector.cameraModel, "Future camera");
            compare(selector.lensModel, "");
            const brand = findChild(selector, "cameraBrandSelector");
            brand.currentIndex = 2;
            brand.activated(2);
            compare(selector.cameraBrand, "Other maker");
            compare(selector.cameraModel, "");
        }

        function test_review_hide_and_restore_without_upload_or_automatic_load() {
            const browser = createTemporaryObject(browserComponent, scene);
            verify(browser);
            browser.selector.setSelection("Sony", "ILCE-6000", "Sony E 16-50mm");
            compare(browser.profiles.length, 3);
            compare(controllerObject.loaded, "");
            browser.preview(0);
            compare(controllerObject.loaded, "profile1");
            browser.preview(1);
            compare(controllerObject.loaded, "profile2");
            browser.setHidden(true);
            compare(browser.profiles.length, 2);
            compare(browser.hiddenCount, 1);
            compare(controllerObject.loaded, "profile3");
            verify(JSON.parse(settingsObject.values.lensProfileHidden).crc2);
            // Recreate the component to verify persistence, then restore through
            // the same control used in the application.
            browser.destroy();
            const restored = createTemporaryObject(browserComponent, scene);
            restored.selector.setSelection("Sony", "ILCE-6000", "Sony E 16-50mm");
            compare(restored.profiles.length, 2);
            const hiddenToggle = findChild(restored, "showHiddenProfiles");
            hiddenToggle.checked = true;
            compare(restored.profiles.length, 3);
            restored.preview(1);
            restored.setHidden(false);
            compare(restored.hiddenCount, 0);
            verify(!JSON.parse(settingsObject.values.lensProfileHidden).crc2);
        }
    }
}
