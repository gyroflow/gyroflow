// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Dialogs

import "../components/"

MenuItem {
    id: root;
    text: qsTr("Lens profile");
    iconName: "lens";
    objectName: "lens";

    property int calibWidth: 0;
    property int calibHeight: 0;

    property int videoWidth: 0;
    property int videoHeight: 0;

    property real input_horizontal_stretch: 1;
    property real input_vertical_stretch: 1;

    property real cropFactor: 0;

    property bool lensProfilesListPrepared: false;
    property var distortionCoeffs: [];
    property string profileName;
    property string profileOriginalJson;
    property string profileChecksum;
    property string profilePath;

    property bool fetched_from_github: false;
    property bool selected_manually: false;
    property var cameraBrandModel: [qsTr("Any brand"), qsTr("Other")];
    property var cameraModelModel: [qsTr("Any model"), qsTr("Other")];
    property var cameraLensModel: [qsTr("Any lens"), qsTr("Other")];
    property var compatibleCameras: [];
    property var detectedCamera: ({});
    property var reviewProfiles: [];
    property int reviewIndex: -1;
    property var activeProfileLoad: null;
    property bool presetLoadBusy: false;
    property var queuedProfileLoad: null;
    property bool suppressSearchUpdate: false;
    property var rejectedProfiles: ({});
    // `restoreLensProfile` shows the profile it puts back, but doesn't apply its settings again or close the panel
    property bool restoring: false;

    property bool isSony: false;

    FileDialog {
        id: fileDialog;
        property var extensions: ["json"];

        title: qsTr("Choose a lens profile")
        nameFilters: [qsTr("Lens profiles") + " (*.json" + (Qt.platform.os == "ios"? " *.txt" : "") + ")"];
        type: "lens";
        onAccepted: loadFile(fileDialog.selectedFile);
    }
    Timer {
        id: profileLoadExpiry;
        interval: 15000;
        onTriggered: {
            if (root.presetLoadBusy) {
                restart();
            } else {
                root.activeProfileLoad = null;
            }
        }
    }
    function loadFile(url: url): void {
        root.selected_manually = true;
        root.activeProfileLoad = null;
        profileLoadExpiry.stop();
        controller.load_lens_profile(url.toString());
    }

    function loadGyroflow(obj: var): void {
        if (typeof obj.light_refraction_coefficient !== "undefined") {
            isUnderwater.checked = Math.round(+obj.light_refraction_coefficient * 1000) == 1330;
        }
        const stab = obj.stabilization || { };
        if (stab.hasOwnProperty("lens_metadata_delay_frames")) {
            lensDelay.value = +stab.lens_metadata_delay_frames;
        }
        if (stab.hasOwnProperty("lens_breathing_enabled")) {
            lensBreathing.checked = !!stab.lens_breathing_enabled;
        }
        // The core settled it on import: a project from before it draws rectilinear, a preset without it leaves it alone
        outputProjection.setValue(controller.output_projection);
    }

    Component.onCompleted: {
        root.loadRejectedProfiles();
        controller.load_profiles(true);

        QT_TRANSLATE_NOOP("TableList", "Camera");
        QT_TRANSLATE_NOOP("TableList", "Lens");
        QT_TRANSLATE_NOOP("TableList", "Setting");
        QT_TRANSLATE_NOOP("TableList", "Additional info");
        QT_TRANSLATE_NOOP("TableList", "Dimensions");
        QT_TRANSLATE_NOOP("TableList", "Calibrated by");
        QT_TRANSLATE_NOOP("TableList", "Focal length");
        QT_TRANSLATE_NOOP("TableList", "Crop factor");
        QT_TRANSLATE_NOOP("TableList", "Asymmetrical");
        QT_TRANSLATE_NOOP("TableList", "Distortion model");
        QT_TRANSLATE_NOOP("TableList", "Digital lens");
        QT_TRANSLATE_NOOP("TableList", "Output projection");
    }
    Timer {
        id: profilesUpdateTimer;
        interval: 1000;
        property bool fromDisk: true;
        onTriggered: controller.load_profiles(fromDisk);
    }
    Connections {
        target: controller;
        function onLens_metadata_delay_changed(): void {
            lensDelay.value = controller.lens_metadata_delay_frames;
        }
        function onLens_delay_estimated(delay_frames: int, correlation: real): void {
            if (correlation > 0) {
                lensDelay.value = delay_frames;
            } else {
                messageBox(Modal.Warning, qsTr("The lens metadata delay could not be estimated. The clip needs a zoom recorded in the lens metadata, and the picture has to follow it clearly."), [ { text: qsTr("Ok"), accent: true } ]);
            }
        }
        function onAll_profiles_loaded(): void {
            if (!lensProfilesListPrepared) { // If it's the first load
                controller.request_profile_ratings();
            }

            const previous = {
                brand: selectorValue(cameraBrand),
                model: selectorValue(cameraModel),
                lens: selectorValue(cameraLens)
            };
            const firstLoad = !lensProfilesListPrepared;
            lensProfilesListPrepared = true;
            root.refreshCameraBrands();
            if (firstLoad) {
                root.applyDetectedCamera();
            } else {
                setSelectorValue(cameraBrand, previous.brand);
                root.refreshCameraModels();
                setSelectorValue(cameraModel, previous.model);
                root.refreshCameraLenses();
                setSelectorValue(cameraLens, previous.lens);
                root.searchProfiles();
            }

            root.loadFavorites();
            if (!root.fetched_from_github) {
                root.fetched_from_github = true;
                controller.fetch_profiles_from_github();
            }
        }
        function onLens_profiles_updated(fromDisk: bool): void {
            profilesUpdateTimer.fromDisk = fromDisk;
            profilesUpdateTimer.start();
        }
        function onTelemetry_loaded(is_main_video: bool, filename: string, camera: string, additional_data: var): void {
            root.isSony = (camera || "").startsWith("Sony");
            if (is_main_video) {
                // Every file starts from the defaults (see `StabilizationParams::clear`): breathing compensation on,
                // no lens metadata delay. A project file loaded afterwards overrides them through loadGyroflow.
                lensBreathing.checked = controller.lens_breathing_enabled;
                lensDelay.value = controller.lens_metadata_delay_frames;
                outputProjection.setValue(controller.output_projection);
            }
            // Keep the focal-length-only profile aligned with the loaded clip. Per-frame focal metadata already
            // provides the correction and must not be combined with a generated profile.
            if (focalLengthOnly.checked) {
                if (controller.has_per_frame_focal_length) {
                    root.restoreLensProfile();
                } else if (is_main_video) {
                    focalLengthOnly.inAdvanced = false;
                    controller.load_focal_length_profile(focalLength.value);
                }
            }
            if (!is_main_video) {
                return;
            }

            const identity = additional_data && additional_data.camera_identifier;
            root.detectedCamera = identity && identity.brand? {
                "brand": identity.brand,
                "model": identity.model || "",
                "lens": identity.lens_model || identity.lens_info || ""
            } : {};
            if (root.lensProfilesListPrepared) root.applyDetectedCamera();
        }
        function onSearch_lens_profile_finished(profiles: list<var>): void {
            root.reviewProfiles = profiles;
            root.reviewIndex = profiles.findIndex(item =>
                item[1] === root.profilePath || (!!root.profileChecksum && item[2] === root.profileChecksum));
        }
        function onGyroflow_file_loaded(obj: var): void {
            if (!root.presetLoadBusy) return;

            root.presetLoadBusy = false;
            root.activeProfileLoad = null;
            profileLoadExpiry.stop();

            const pending = root.queuedProfileLoad;
            root.queuedProfileLoad = null;
            if (pending) Qt.callLater(root.loadProfileItem, pending);
        }
        function onLens_profile_loaded(json_str: string, filepath: string, checksum: string): void {
            if (json_str) {
                const obj = JSON.parse(json_str);
                if (obj) {
                    const load = root.activeProfileLoad || {};
                    const profileFile = load.file || filepath;
                    const profileCrc = load.checksum || checksum;
                    root.activeProfileLoad = null;
                    if (!root.presetLoadBusy) profileLoadExpiry.stop();
                    let lensInfo = {
                        "Camera":          obj.camera_brand + " " + obj.camera_model,
                        "Lens":            obj.lens_model,
                        "Setting":         obj.camera_setting,
                        "Additional info": obj.note,
                        "Dimensions":      obj.calib_dimension.w + "x" + obj.calib_dimension.h,
                        "Calibrated by":   obj.calibrated_by
                    };

                    if (+obj.focal_length > 0) lensInfo["Focal length"] = obj.focal_length.toFixed(2) + " mm";
                    if (+obj.crop_factor  > 0) lensInfo["Crop factor"]  = obj.crop_factor.toFixed(2) + "x";
                    if (obj.asymmetrical) lensInfo["Asymmetrical"] = qsTr("Yes");
                    if (obj.distortion_model && obj.distortion_model != "opencv_fisheye") lensInfo["Distortion model"] = obj.distortion_model;
                    if (obj.digital_lens) lensInfo["Digital lens"] = obj.digital_lens;
                    lensInfo["Output projection"] = outputProjection.text;

                    const focalLengthOnlyMm = root.focalLengthOnlyOf(obj);
                    focalLengthOnly.sync(focalLengthOnlyMm, !!obj.official);
                    if (focalLengthOnlyMm > 0) { // The checkbox tells the rest
                        lensInfo = { "Dimensions": lensInfo["Dimensions"], "Output projection": lensInfo["Output projection"] };
                    }

                    info.model = lensInfo;

                    root.cropFactor = +obj.crop_factor;

                    if (!root.selected_manually && !root.restoring &&
                           (obj.calibrated_by == "Eddy" ||
                            obj.calibrated_by == "GoPro" ||
                            obj.calibrated_by == "DJI" ||
                            obj.calibrated_by == "Xtra" ||
                            obj.calibrated_by == "Insta360" ||
                            obj.calibrated_by == "Canon" ||
                            obj.calibrated_by == "Sony")) {
                        root.opened = false;
                        window.motionData.opened = false;
                    }

                    officialInfo.show = !obj.official && !settings.value("rated-profile-" + profileCrc, false);
                    officialInfo.canRate = true;
                    officialInfo.thankYou = false;
                    root.profileName = (profileFile || obj.name || "").replace(/^.*?[\/\\]([^\/\\]+?)$/, "$1");
                    root.profileOriginalJson = json_str;
                    root.profileChecksum = profileCrc;
                    root.profilePath = profileFile;

                    if (!root.restoring) {
                        if (obj.output_dimension && obj.output_dimension.w > 0 && (window.exportSettings.outWidth != obj.output_dimension.w || window.exportSettings.outHeight != obj.output_dimension.h)) {
                            Qt.callLater(window.exportSettings.lensProfileLoaded, obj.output_dimension.w, obj.output_dimension.h);
                        }
                        if (+obj.frame_readout_time && Math.abs(+obj.frame_readout_time) > 0) {
                            window.stab.setFrameReadoutTime(obj.frame_readout_time, obj.frame_readout_direction);
                        }
                        if (+obj.gyro_lpf && Math.abs(+obj.gyro_lpf) > 0) {
                            window.motionData.setGyroLpf(obj.gyro_lpf);
                        }
                        if (obj.sync_settings && Object.keys(obj.sync_settings).length > 0) {
                            window.sync.loadGyroflow({
                                synchronization: obj.sync_settings
                            });
                        }
                    }

                    root.input_horizontal_stretch = obj.input_horizontal_stretch > 0.01? obj.input_horizontal_stretch : 1.0;
                    root.input_vertical_stretch   = obj.input_vertical_stretch   > 0.01? obj.input_vertical_stretch   : 1.0;

                    root.calibWidth  = obj.calib_dimension.w / root.input_horizontal_stretch;
                    root.calibHeight = obj.calib_dimension.h / root.input_vertical_stretch;
                    const coeffs = obj.fisheye_params.distortion_coeffs;
                    root.distortionCoeffs = coeffs;
                    const mtrx = obj.fisheye_params.camera_matrix;
                    k1.setInitialValue(coeffs[0] || 0.0);
                    k2.setInitialValue(coeffs[1] || 0.0);
                    k3.setInitialValue(coeffs[2] || 0.0);
                    k4.setInitialValue(coeffs[3] || 0.0);
                    fx.setInitialValue(mtrx[0][0]);
                    fy.setInitialValue(mtrx[1][1]);
                    cx.setInitialValue(mtrx[0][2]);
                    cy.setInitialValue(mtrx[1][2]);

                    // Set asymmetrical lens center bias
                    /*if (obj.asymmetrical) {
                        console.log(-((mtrx[0][2] / (obj.calib_dimension.w / 2.0)) - 1.0));
                        console.log(-((mtrx[1][2] / (obj.calib_dimension.h / 2.0)) - 1.0));
                    }*/
                    // If focal length in pixels is large, it's more likely that Almeida pose estimator will yield better results
                    if (mtrx[0][0] > 10000 && !root.restoring) {
                        window.sync.poseMethod.currentIndex = 1; // Almeida
                    }
                }
                Qt.callLater(controller.recompute_threaded);
            } else { // `controller.clear_lens_profile`, or a project saved with none
                root.lensCleared();
            }
        }
    }

    // The 35 mm equivalent focal length of a profile without any distortion (`controller.load_focal_length_profile`
    // makes them), or 0 for a calibrated one
    function focalLengthOnlyOf(obj: var): real {
        const coeffs = obj.fisheye_params?.distortion_coeffs || [];
        const mtrx   = obj.fisheye_params?.camera_matrix || [];
        if (obj.distortion_model != "opencv_standard" || coeffs.some(x => x != 0) || mtrx.length != 3 || obj.interpolations) return 0;
        const w = obj.calib_dimension.w, h = obj.calib_dimension.h;
        return mtrx[0][0] * Math.sqrt(36*36 + 24*24) / Math.max(1, Math.sqrt(w*w + h*h));
    }
    // Unticked: the unofficial profile it was ticked over comes back (`controller.restore_lens_profile`), or no profile
    function restoreLensProfile(): void {
        root.restoring = true;
        controller.restore_lens_profile();
        root.restoring = false;
        if (!controller.lens_loaded) root.lensCleared();
    }
    function lensCleared(): void {
        focalLengthOnly.syncNone();
        info.model = ({});
        officialInfo.show = false;
        root.calibWidth = 0;
        root.calibHeight = 0;
        root.cropFactor = 0;
        root.input_horizontal_stretch = 1;
        root.input_vertical_stretch = 1;
    }

    property int currentVideoAspectRatio: Math.round((root.videoWidth / Math.max(1, root.videoHeight)) * 1000);
    property int currentVideoAspectRatioSwapped: Math.round((root.videoHeight / Math.max(1, root.videoWidth)) * 1000);

    property var favorites: ({});
    function loadFavorites(): void {
        const list = settings.value("lensProfileFavorites", "");
        let fav = {};
        for (const x of list.split(",")) {
            if (x)
                fav[x] = 1;
        }
        favorites = fav;
    }
    function updateFavorites(): void {
        settings.setValue("lensProfileFavorites", Object.keys(favorites).filter(v => v).join(","));
    }
    function loadRejectedProfiles(): void {
        const stored = settings.value("hiddenLensProfiles", "[]");
        let values = [];
        try {
            values = JSON.parse(stored || "[]");
        } catch (e) {
            values = (stored || "").split(",");
        }
        let rejected = {};
        for (const value of values) {
            if (value) {
                rejected[value] = 1;
            }
        }
        rejectedProfiles = rejected;
    }
    function updateRejectedProfiles(): void {
        settings.setValue("hiddenLensProfiles", JSON.stringify(Object.keys(rejectedProfiles).filter(v => v)));
    }
    function rejectProfile(checksum: string, path: string): void {
        const identities = [checksum, path].filter((value, index, values) => value && values.indexOf(value) === index);
        if (!identities.length) return;

        let rejected = Object.assign({}, rejectedProfiles);
        for (const identity of identities) rejected[identity] = 1;
        rejectedProfiles = rejected;
        updateRejectedProfiles();

        root.reviewProfiles = root.reviewProfiles.filter(item => {
            const sameChecksum = checksum && item[2] === checksum;
            const samePath = path && item[1] === path;
            return !sameChecksum && !samePath;
        });
        if (root.reviewIndex >= root.reviewProfiles.length) {
            root.reviewIndex = root.reviewProfiles.length - 1;
        }
        root.searchProfiles();
    }
    function rejectCurrentProfile(submitRating: bool): void {
        if (submitRating && root.profileChecksum && root.profileOriginalJson) {
            controller.rate_profile(root.profileName, root.profileOriginalJson, root.profileChecksum, false);
        }
        root.rejectProfile(root.profileChecksum, root.profilePath);
        officialInfo.thankYou = true;
        officialInfo.canRate = false;
        tyTimer.start();
    }

    function selectorModel(emptyText: string, values: var): var {
        let model = [emptyText];
        for (const value of values || []) {
            if (value && model.indexOf(value) === -1) {
                model.push(value);
            }
        }
        model.push(qsTr("Other"));
        return model;
    }
    function selectorValue(combo: var): string {
        if (combo.currentIndex <= 0 || combo.currentIndex >= combo.model.length - 1) {
            return "";
        }
        return combo.model[combo.currentIndex];
    }
    function setSelectorValue(combo: var, value: string): bool {
        const index = combo.model.indexOf(value);
        if (index > 0) {
            combo.currentIndex = index;
            return true;
        }
        return false;
    }
    function resetCameraSelection(): void {
        cameraBrand.popup.maxItemWidth = 0;
        cameraModel.popup.maxItemWidth = 0;
        cameraLens.popup.maxItemWidth = 0;
        cameraBrand.currentIndex = 0;
        cameraModelModel = selectorModel(qsTr("Any model"), []);
        cameraModel.currentIndex = 0;
        cameraLensModel = selectorModel(qsTr("Any lens"), []);
        cameraLens.currentIndex = 0;
        compatibleCameras = [];
    }
    function refreshCameraBrands(): void {
        cameraBrandModel = selectorModel(qsTr("Any brand"), controller.camera_database_brands());
        resetCameraSelection();
    }
    function refreshCameraModels(): void {
        const brand = selectorValue(cameraBrand);
        cameraModel.popup.maxItemWidth = 0;
        cameraLens.popup.maxItemWidth = 0;
        cameraModelModel = selectorModel(qsTr("Any model"), brand? controller.camera_database_models(brand) : []);
        cameraModel.currentIndex = 0;
        cameraLensModel = selectorModel(qsTr("Any lens"), []);
        cameraLens.currentIndex = 0;
        compatibleCameras = [];
    }
    function refreshCameraLenses(): void {
        const brand = selectorValue(cameraBrand);
        const model = selectorValue(cameraModel);
        cameraLens.popup.maxItemWidth = 0;
        cameraLensModel = selectorModel(qsTr("Any lens"), brand && model? controller.camera_database_lenses(brand, model) : []);
        cameraLens.currentIndex = 0;
        compatibleCameras = brand && model? controller.camera_database_compatible_models(brand, model) : [];
    }
    function updateCameraSearch(): void {
        searchProfiles();
    }
    function searchProfiles(): void {
        const text = search.text.trim();
        controller.search_lens_profile_for_camera(
            selectorValue(cameraBrand),
            selectorValue(cameraModel),
            selectorValue(cameraLens),
            text,
            Object.keys(root.rejectedProfiles),
            Object.keys(root.favorites),
            root.currentVideoAspectRatio,
            root.currentVideoAspectRatioSwapped
        );
    }
    function compatibleCameraSummary(): string {
        const visible = root.compatibleCameras.slice(0, 4);
        let summary = visible.join(", ");
        if (root.compatibleCameras.length > visible.length) {
            summary += qsTr(" +%1 more").arg(root.compatibleCameras.length - visible.length);
        }
        return qsTr("Compatible: %1").arg(summary);
    }
    function selectorPopupWidth(combo: var): real {
        const parentWidth = combo.parent? combo.parent.width : combo.width;
        const desiredWidth = Math.max(combo.width, combo.popup.maxItemWidth + 16 * dpiScale);
        return Math.min(desiredWidth, parentWidth);
    }
    function selectorPopupX(combo: var): real {
        const parentWidth = combo.parent? combo.parent.width : combo.width;
        return Math.max(-combo.x, Math.min(0, parentWidth - combo.x - combo.popup.width));
    }
    function applyDetectedCamera(): void {
        if (!lensProfilesListPrepared) return;

        resetCameraSelection();
        if (!detectedCamera.brand) {
            searchProfiles();
            return;
        }
        const resolvedCamera = controller.camera_database_resolve_camera(detectedCamera.brand, detectedCamera.model);
        const brand = resolvedCamera.brand || detectedCamera.brand;
        const model = resolvedCamera.model || detectedCamera.model;

        if (setSelectorValue(cameraBrand, brand)) {
            cameraModelModel = selectorModel(qsTr("Any model"), controller.camera_database_models(brand));
            if (model) {
                setSelectorValue(cameraModel, model);
                cameraLensModel = selectorModel(qsTr("Any lens"), controller.camera_database_lenses(brand, model));
                compatibleCameras = controller.camera_database_compatible_models(brand, model);
                if (detectedCamera.lens) {
                    setSelectorValue(cameraLens, detectedCamera.lens);
                }
            }
        }
        updateCameraSearch();
    }
    function loadProfileItem(item: var): void {
        const position = root.reviewProfiles.findIndex(entry => entry[1] === item[1] && entry[2] === item[2]);
        if (position !== -1) root.reviewIndex = position;

        if (root.presetLoadBusy) {
            root.queuedProfileLoad = item;
            return;
        }

        const lensPathOrId = item[1];
        root.activeProfileLoad = { "file": lensPathOrId, "checksum": item[2] };
        profileLoadExpiry.restart();
        let accepted = false;
        try {
            if (lensPathOrId.endsWith(".gyroflow")) {
                const contents = JSON.parse(controller.get_preset_contents(lensPathOrId));
                root.presetLoadBusy = true;
                accepted = window.videoArea.loadGyroflowData(contents, 0);
            } else {
                root.selected_manually = true;
                accepted = controller.load_lens_profile(lensPathOrId);
            }
        } catch (error) {
            console.warn("Unable to load lens preset:", error);
            messageBox(Modal.Error, qsTr("Unable to load this lens preset."), [ { text: qsTr("Ok") } ]);
        }
        if (!accepted) {
            root.presetLoadBusy = false;
            root.activeProfileLoad = null;
            profileLoadExpiry.stop();
        }
    }

    function reviewRelative(step: int): void {
        if (!reviewProfiles.length) {
            return;
        }

        if (reviewIndex < 0) {
            reviewIndex = step < 0? reviewProfiles.length - 1 : 0;
        } else {
            reviewIndex = (reviewIndex + step + reviewProfiles.length) % reviewProfiles.length;
        }
        loadProfileItem(reviewProfiles[reviewIndex]);
    }

    Grid {
        width: parent.width;
        columns: width >= 600 * dpiScale? 3 : 1;
        columnSpacing: 5 * dpiScale;
        rowSpacing: 8 * dpiScale;

        Label {
            text: qsTr("Camera brand");
            width: (parent.width - (parent.columns - 1) * parent.columnSpacing) / parent.columns;
            spacing: 4 * dpiScale;
            CameraSelector {
                id: cameraBrand;
                model: root.cameraBrandModel;
                width: parent.width;
                font.pixelSize: 12 * dpiScale;
                fieldLabel: qsTr("Camera brand");
                popup.x: root.selectorPopupX(cameraBrand);
                popup.width: root.selectorPopupWidth(cameraBrand);
                popup.height: popup.implicitHeight;
                onActivated: {
                    root.refreshCameraModels();
                    root.updateCameraSearch();
                }
            }
        }
        Label {
            text: qsTr("Camera model");
            width: (parent.width - (parent.columns - 1) * parent.columnSpacing) / parent.columns;
            spacing: 4 * dpiScale;
            CameraSelector {
                id: cameraModel;
                model: root.cameraModelModel;
                width: parent.width;
                font.pixelSize: 12 * dpiScale;
                fieldLabel: qsTr("Camera model");
                popup.x: root.selectorPopupX(cameraModel);
                popup.width: root.selectorPopupWidth(cameraModel);
                popup.height: popup.implicitHeight;
                onActivated: {
                    root.refreshCameraLenses();
                    root.updateCameraSearch();
                }
            }
        }
        Label {
            text: qsTr("Lens model");
            width: (parent.width - (parent.columns - 1) * parent.columnSpacing) / parent.columns;
            spacing: 4 * dpiScale;
            CameraSelector {
                id: cameraLens;
                model: root.cameraLensModel;
                width: parent.width;
                font.pixelSize: 12 * dpiScale;
                fieldLabel: qsTr("Lens model");
                popup.x: root.selectorPopupX(cameraLens);
                popup.width: root.selectorPopupWidth(cameraLens);
                popup.height: popup.implicitHeight;
                onActivated: root.updateCameraSearch();
            }
        }
    }
    BasicText {
        visible: root.compatibleCameras.length > 0;
        width: parent.width;
        text: root.compatibleCameraSummary();
        color: Qt.darker(styleTextColor, 1.25);
        font.pixelSize: 11 * dpiScale;
        elide: Text.ElideRight;
    }

    SearchField {
        id: search;
        placeholderText: qsTr("Search...");
        height: 25 * dpiScale;
        width: parent.width;
        topPadding: 5 * dpiScale;
        profilesMenu: root;
        autoSearch: false;
        popup.x: 0;
        popup.itemHeight: 30 * dpiScale;
        popup.width: width;
        popup.height: Math.min(popup.implicitHeight, 8 * popup.itemHeight + 4 * dpiScale);
        onTextChanged: if (!root.suppressSearchUpdate) root.searchProfiles();
        onSelected: (item) => {
            root.suppressSearchUpdate = true;
            root.loadProfileItem(item);
            Qt.callLater(() => root.suppressSearchUpdate = false);
        }
        popup.lv.delegate: LensProfileSearchDelegate {
            popup: search.popup;
            profilesMenu: root;
        }
    }
    Row {
        visible: root.reviewProfiles.length > 1 || ((root.profileChecksum || root.profilePath) && root.reviewProfiles.length > 0);
        anchors.horizontalCenter: parent.horizontalCenter;
        spacing: 5 * dpiScale;

        Button {
            iconName: "chevron-left";
            text: "";
            tooltip: qsTr("Previous profile");
            width: 35 * dpiScale;
            leftPadding: 5 * dpiScale;
            rightPadding: 5 * dpiScale;
            enabled: root.reviewProfiles.length > 1;
            onClicked: root.reviewRelative(-1);
        }
        BasicText {
            text: (root.reviewIndex >= 0? (root.reviewIndex + 1) : 0) + "/" + root.reviewProfiles.length;
            height: 35 * dpiScale;
            verticalAlignment: Text.AlignVCenter;
        }
        Button {
            iconName: "chevron-right";
            text: "";
            tooltip: qsTr("Next profile");
            width: 35 * dpiScale;
            leftPadding: 5 * dpiScale;
            rightPadding: 5 * dpiScale;
            enabled: root.reviewProfiles.length > 1;
            onClicked: root.reviewRelative(1);
        }
        Button {
            iconName: "close";
            text: "";
            tooltip: qsTr("Hide profile locally");
            width: 35 * dpiScale;
            leftPadding: 5 * dpiScale;
            rightPadding: 5 * dpiScale;
            enabled: !!(root.profileChecksum || root.profilePath);
            onClicked: root.rejectCurrentProfile(false);
        }
    }
    Button {
        visible: Object.keys(root.rejectedProfiles).length > 0;
        anchors.horizontalCenter: parent.horizontalCenter;
        text: qsTr("Restore hidden profiles");
        onClicked: {
            root.rejectedProfiles = {};
            root.updateRejectedProfiles();
            root.searchProfiles();
        }
    }
    Row {
        anchors.horizontalCenter: parent.horizontalCenter;
        spacing: 10 * dpiScale;
        Button {
            text: qsTr("Open file");
            iconName: "file-empty"
            onClicked: fileDialog.open2();
        }
        Button {
            text: qsTr("Create new");
            iconName: "plus";
            icon.width: 15 * dpiScale;
            icon.height: 15 * dpiScale;
            property var calibratorWnd: null;
            onClicked: {
                if (!calibratorWnd) {
                    ui_tools.init_calibrator();
                    calibratorWnd = Qt.createComponent("../Calibrator.qml").createObject(main_window)
                    calibratorWnd.show();
                    calibratorWnd.closing.connect(function(e) {
                        calibratorWnd.destroy();
                        calibratorWnd = null;
                    })
                }
            }
        }
    }

    // The checkbox below is here for a file with no profile, and in Advanced (`focalLengthOnlyAdvanced`) over an unofficial one
    Item {
        id: focalLengthOnlyMain;
        width: parent.width;
        height: childrenRect.height;
        visible: focalLengthOnly.shown && !focalLengthOnly.inAdvanced;
    }
    CheckBoxWithContent {
        id: focalLengthOnly;
        parent: focalLengthOnly.inAdvanced? focalLengthOnlyAdvanced : focalLengthOnlyMain;
        text: qsTr("No distortion, focal length only");
        tooltip: qsTr("Instead of a lens profile, use a lens without any distortion, of this focal length.") + "\n" +
                 qsTr("Use only if your video is already undistorted (via a rectilinear lens or in-camera processing).");
        // Offered with no profile, over an unofficial one (in Advanced) and over its own, which stays where it was ticked.
        // Set by `sync` rather than bound to the lens: just unticked, it's still over its own profile until the one it
        // puts back is loaded, and mustn't hide or move under the click
        property bool offered: true;
        property bool inAdvanced: false;
        // With no profile, the focal length the lens metadata has for every frame is used, with no distortion. Where it
        // has only millimetres with no sensor size to turn them into pixels, a generated profile would scale them against
        // its 35 mm equivalent (`FrameTransform::get_lens_data_at_lens_timestamp`), wrong on any smaller sensor
        readonly property bool shown: window.videoArea.vid.loaded && !controller.loading_gyro_in_progress && !controller.has_per_frame_focal_length && offered;
        property bool syncing: false;
        cb.onCheckedChanged: {
            if (focalLengthOnly.syncing) return;
            if (focalLengthOnly.checked) controller.load_focal_length_profile(focalLength.value);
            else root.restoreLensProfile();
        }
        // Shows the lens that's loaded (`focalLengthOnlyOf`), without loading anything: a generated one of this focal
        // length, or for 0 a calibrated one, which has it in Advanced when it's unofficial and nowhere when it's official
        function sync(focal_length: real, official: bool): void {
            focalLengthOnly.syncing = true;
            if (focal_length > 0) {
                // Ticked by a project or for the file before, not over a profile here: unticking leaves none
                if (!focalLengthOnly.checked) focalLengthOnly.inAdvanced = false;
                focalLengthOnly.offered = true;
                // The value comes back from the profile while it's being typed: only rounding apart, it's left alone
                if (Math.abs(focalLength.value - focal_length) > 1e-6) focalLength.value = focal_length;
            } else {
                focalLengthOnly.offered = !official;
                focalLengthOnly.inAdvanced = true;
            }
            focalLengthOnly.checked = focal_length > 0;
            focalLengthOnly.syncing = false;
        }
        function syncNone(): void {
            focalLengthOnly.syncing = true;
            focalLengthOnly.offered = true;
            focalLengthOnly.inAdvanced = false;
            focalLengthOnly.checked = false;
            focalLengthOnly.syncing = false;
        }

        Label {
            position: Label.LeftPosition;
            text: qsTr("Focal length (35 mm equiv.)");
            tooltip: qsTr("The focal length with the same field of view on a full frame (36x24 mm) sensor: the focal length of the lens multiplied by the crop factor of the sensor.");
            NumberField {
                id: focalLength;
                width: parent.width;
                height: 25 * dpiScale;
                unit: qsTr("mm");
                precision: 1;
                value: 24;
                from: 0.1;
                onValueChanged: if (focalLengthOnly.checked && !focalLengthOnly.syncing) focalLengthTimer.restart();
            }
        }
        Timer {
            id: focalLengthTimer;
            interval: 200;
            onTriggered: if (focalLengthOnly.checked) controller.load_focal_length_profile(focalLength.value);
        }
    }

    InfoMessageSmall {
        id: officialInfo;
        type: InfoMessage.Warning;
        show: false;
        property bool canRate: true;
        property bool thankYou: false;
        text: qsTr("This lens profile is unofficial, we can't guarantee its correctness. Use at your own risk.") + (canRate? "<br>" +
              qsTr("Rate this profile: [Good] | [Bad]")
              .replace(/\[(.*?)\]/, "<a href=\"#good\">$1</a>")
              .replace(/\[(.*?)\]/, "<a href=\"#bad\">$1</a>") : (thankYou? "<br>" + qsTr("Thank you for rating this profile.") : ""));

        MouseArea {
            anchors.fill: parent;
            cursorShape: parent.t.hoveredLink? Qt.PointingHandCursor : Qt.ArrowCursor;
            acceptedButtons: Qt.NoButton;
        }
        Connections {
            target: officialInfo.t;
            function onLinkActivated(link: url): void {
                const isGood = link === "#good";
                controller.rate_profile(root.profileName, root.profileOriginalJson, root.profileChecksum, isGood);
                if (isGood) {
                    settings.setValue("rated-profile-" + root.profileChecksum, true);
                } else {
                    root.rejectCurrentProfile(false);
                }
                officialInfo.thankYou = true;
                officialInfo.canRate = false;
                tyTimer.start();
            }
        }
        Timer {
            id: tyTimer;
            interval: 5000;
            onTriggered: officialInfo.thankYou = false;
        }
    }

    InfoMessageSmall {
        type: lensRatio != videoRatio? InfoMessage.Error : InfoMessage.Warning;
        show: root.calibWidth > 0 && root.videoWidth > 0 && (root.calibWidth != root.videoWidth || root.calibHeight != root.videoHeight);
        property string lensRatio: (root.calibWidth / Math.max(1, root.calibHeight)).toFixed(3);
        property string videoRatio: (root.videoWidth / Math.max(1, root.videoHeight)).toFixed(3);
        text: lensRatio != videoRatio? qsTr("Lens profile aspect ratio doesn't match the file aspect ratio. The result will not look correct.") :
                                       qsTr("Lens profile dimensions don't match the file dimensions. The result may not look correct.");
    }

    TableList {
        id: info;
        copyable: true;
        model: ({ })
        editableFields: ({
            "Output projection": {
                "type": "choice",
                "options": outputProjection.options,
                "tooltips": outputProjection.tooltips,
                "value": function() { return outputProjection.currentIndex; },
                "onChange": function(index) { outputProjection.currentIndex = index; }
            }
        });
    }

    QtObject {
        id: outputProjection;
        // Index -> `OutputProjection` as i32, -1 is "Automatic" (`projection::AUTOMATIC`)
        property var values: [0, 1, 2, 3, 4, -1];
        property var options: [QT_TRANSLATE_NOOP("Popup", "Rectilinear"), QT_TRANSLATE_NOOP("Popup", "Stereographic"), QT_TRANSLATE_NOOP("Popup", "Equidistant"), QT_TRANSLATE_NOOP("Popup", "Equisolid angle"), QT_TRANSLATE_NOOP("Popup", "Orthographic"), QT_TRANSLATE_NOOP("Popup", "Automatic")];
        property var tooltips: ([
            qsTr("Straight lines stay straight. A lens that sees more than 180° can't be shown whole, so its edges are cropped."),
            qsTr("Fisheye that keeps shapes natural but bends straight lines. Shows the whole picture of a lens wider than 180°."),
            qsTr("Fisheye where the distance from the center is proportional to the angle. Shows the whole picture of a lens wider than 180°."),
            qsTr("Fisheye that keeps areas in proportion. Shows the whole picture of a lens wider than 180°."),
            qsTr("Fisheye that compresses the edges strongly. Shows at most 180°."),
            qsTr("Rectilinear, or stereographic when the lens sees more than 180°.")
        ]);
        property int currentIndex: 0;
        property string text: qsTranslate("Popup", options[currentIndex]);
        function setValue(v: int): void {
            const i = values.indexOf(v);
            currentIndex = i >= 0? i : 0;
        }
        onCurrentIndexChanged: controller.output_projection = values[currentIndex];
        onTextChanged: if (info.model.hasOwnProperty("Output projection")) info.updateEntry("Output projection", text);
    }

    CheckBox {
        id: lensBreathing;
        text: qsTr("Lens breathing compensation");
        visible: controller.has_lens_breathing;
        tooltip: qsTr("Keeps the field of view constant while focusing, using the lens breathing data recorded by the camera.");
        checked: controller.lens_breathing_enabled;
        onCheckedChanged: controller.lens_breathing_enabled = checked;
    }

    AdvancedSection {
        btn.text: qsTr("Advanced");
        visible: Object.keys(info.model).length > 0

        Item {
            id: focalLengthOnlyAdvanced;
            width: parent.width;
            height: childrenRect.height;
            visible: focalLengthOnly.shown && focalLengthOnly.inAdvanced;
        }
        CheckBox {
            id: isUnderwater;
            text: qsTr("Lens is under water");
            checked: false;
            tooltip: qsTr("Enable if you're filming under water. This will adjust the refraction coefficient.");
            property bool keyframesEnabled: false;

            onCheckedChanged: {
                controller.light_refraction_coefficient = checked? 1.33 : 1.0;
                if (keyframesEnabled) {
                    controller.set_keyframe("LightRefractionCoeff", window.videoArea.timeline.getTimestampUs(), checked? 1.33 : 1.0);
                }
            }
            ContextMenuMouseArea {
                cursorShape: Qt.ibeam;
                underlyingItem: isUnderwater;
                onContextMenu: (isHold, x, y) => menuLoader.popup(isUnderwater, x, y);
            }

            Component {
                id: isUnderwaterMenu;
                Menu {
                    font.pixelSize: 11.5 * dpiScale;
                    Action {
                        iconName: "keyframe";
                        text: qsTr("Enable keyframing");
                        checked: isUnderwater.keyframesEnabled;
                        onTriggered: {
                            checked = !checked;
                            isUnderwater.keyframesEnabled = checked;
                            if (!checked) {
                                controller.clear_keyframes_type("LightRefractionCoeff");
                            }
                        }
                    }
                    Action {
                        iconName: "plus";
                        enabled: isUnderwater.keyframesEnabled;
                        text: qsTr("Add keyframe");
                        onTriggered: controller.set_keyframe("LightRefractionCoeff", window.videoArea.timeline.getTimestampUs(), isUnderwater.checked? 1.33 : 1.0);
                    }
                }
            }
            ContextMenuLoader {
                id: menuLoader;
                sourceComponent: isUnderwaterMenu
            }
        }

        Label {
            text: qsTr("Lens metadata delay");
            position: Label.LeftPosition;
            visible: root.isSony;
            tooltip: qsTr("How many frames later than the picture the lens reports its focal length. It depends on the lens, not on the frame rate: some report two frames late.") + "\n" +
                     qsTr("\"Analyze\" measures it on the zooms recorded in this clip. Positive values read the lens metadata later.");
            Row {
                width: parent.width;
                spacing: 5 * dpiScale;
                NumberField {
                    id: lensDelay;
                    from: -30;
                    to: 30;
                    unit: qsTr("frames");
                    precision: 0;
                    font.pixelSize: 11 * dpiScale;
                    width: parent.width - analyzeLensDelay.width - parent.spacing;
                    anchors.verticalCenter: parent.verticalCenter;
                    value: controller.lens_metadata_delay_frames;
                    defaultValue: 0;
                    onValueChanged: controller.lens_metadata_delay_frames = Math.round(value);
                }
                LinkButton {
                    id: analyzeLensDelay;
                    text: qsTr("Analyze");
                    leftPadding: 6 * dpiScale;
                    rightPadding: 6 * dpiScale;
                    enabled: !controller.sync_in_progress && window.videoArea.vid.loaded;
                    anchors.verticalCenter: parent.verticalCenter;
                    onClicked: controller.start_autosync("0", window.sync.getSettingsJson(), "estimate_lens_delay");
                }
            }
        }

        component SmallNumberField: NumberField {
            property bool preventChange2: true;
            width: parent.width / 2;
            precision: 12;
            property string param: "  ";
            tooltip: param[0] + "<font size=\"1\">" + param[1] + "</font>"
            font.pixelSize: 11 * dpiScale;
            onValueChanged: {
                if (!preventChange2) controller.set_lens_param(param, value);
            }
            function setInitialValue(v: real): void {
                preventChange2 = true;
                value = v;
                preventChange2 = false;
            }
        }

        Label {
            text: qsTr("Pixel focal length");

            Row {
                spacing: 4 * dpiScale;
                width: parent.width;
                SmallNumberField { id: fx; param: "fx"; }
                SmallNumberField { id: fy; param: "fy"; }
            }
        }
        Label {
            text: qsTr("Focal center");

            Row {
                spacing: 4 * dpiScale;
                width: parent.width;
                SmallNumberField { id: cx; param: "cx"; }
                SmallNumberField { id: cy; param: "cy"; }
            }
        }
        Label {
            text: qsTr("Distortion coefficients");

            Column {
                spacing: 4 * dpiScale;
                width: parent.width;
                Row {
                    spacing: 4 * dpiScale;
                    width: parent.width;
                    SmallNumberField { id: k1; param: "k1"; precision: 16; }
                    SmallNumberField { id: k2; param: "k2"; precision: 16; }
                }
                Row {
                    spacing: 4 * dpiScale;
                    width: parent.width;
                    SmallNumberField { id: k3; param: "k3"; precision: 16; }
                    SmallNumberField { id: k4; param: "k4"; precision: 16; }
                }
            }
        }
        Row {
            anchors.horizontalCenter: parent.horizontalCenter;
            spacing: 5 * dpiScale;
            LinkButton {
                text: qsTr("Export STMap");
                OutputPathField { id: opf; visible: false; }
                enabled: window.videoArea.vid.loaded;
                onClicked: {
                    opf.selectFolder("", function(folder_url) {
                        if (controller.has_per_frame_lens_data()) {
                            messageBox(Modal.Question, qsTr("This file contains per-frame lens metadata. Do you want to export an STMap sequence or a single frame?"), [
                                { text: qsTr("Single frame"), accent: true, clicked: () => { controller.export_stmap(folder_url, false); } },
                                { text: qsTr("STMap sequence"), clicked: () => { controller.export_stmap(folder_url, true); } },
                            ]);
                        } else {
                            controller.export_stmap(folder_url, false);
                        }
                    });
                }

                Connections {
                    target: controller;
                    function onStmap_progress(progress: real, ready: int, total: int): void {
                        window.videoArea.videoLoader.active = progress < 1;
                        window.videoArea.videoLoader.currentFrame = ready;
                        window.videoArea.videoLoader.totalFrames = total;
                        window.videoArea.videoLoader.text = progress < 1? qsTr("Exporting %1...") : "";
                        window.videoArea.videoLoader.progress = progress < 1? progress : -1;
                        window.videoArea.videoLoader.cancelable = true;
                    }
                }
            }
            LinkButton {
                text: qsTr("Clear");
                visible: controller.lens_loaded;
                onClicked: controller.clear_lens_profile();
            }
        }
    }

    DropTarget {
        parent: root.innerItem;
        color: styleBackground2;
        z: 999;
        anchors.rightMargin: -28 * dpiScale;
        anchors.topMargin: 35 * dpiScale;
        anchors.bottomMargin: -35 * dpiScale;
        extensions: fileDialog.extensions;
        onLoadFile: (url) => root.loadFile(url);
    }

    // -------------------------------------------------------------------
    // ---------------------- Maintenance functions ----------------------
    // -------------------------------------------------------------------
    /*
    property int fileno: 0;
    property var files: [
        ... // dir /b | clip
    ];
    Shortcut {
        sequences: ["F8"];
        onActivated: {
            root.fileno = Math.abs(++fileno % files.length);
            console.log(root.fileno);
            controller.load_lens_profile("file:///d:/lens_review/" + root.files[root.fileno]);
        }
    }
    Shortcut {
        sequences: ["F7"];
        onActivated: {
            root.fileno = Math.abs(--fileno % files.length);
            console.log(root.fileno);
            controller.load_lens_profile("file:///d:/lens_review/" + root.files[root.fileno]);
        }
    }
    Shortcut {
        sequences: ["Delete"];
        onActivated: {
            console.log("deleting " + root.files[root.fileno]);
            filesystem.move_to_trash("file:///d:/lens_review/" + root.files[root.fileno]);
        }
    }
    */
}
