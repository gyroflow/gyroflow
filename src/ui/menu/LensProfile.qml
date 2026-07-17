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
    property bool updatingCameraSelectors: false;
    property bool suppressSearchUpdate: false;
    property var cameraBrandModel: [qsTr("Any brand"), qsTr("Other")];
    property var cameraModelModel: [qsTr("Any model"), qsTr("Other")];
    property var cameraLensModel: [qsTr("Any lens"), qsTr("Other")];
    property var compatibleCameras: [];
    property var detectedCamera: ({});
    property var hiddenProfiles: ({});
    property var reviewProfiles: [];
    property int reviewIndex: -1;

    FileDialog {
        id: fileDialog;
        property var extensions: ["json"];

        title: qsTr("Choose a lens profile")
        nameFilters: [qsTr("Lens profiles") + " (*.json" + (Qt.platform.os == "ios"? " *.txt" : "") + ")"];
        type: "lens";
        onAccepted: loadFile(fileDialog.selectedFile);
    }
    function loadFile(url: url): void {
        root.selected_manually = true;
        controller.load_lens_profile(url.toString());
    }

    function loadGyroflow(obj: var): void {
        if (typeof obj.light_refraction_coefficient !== "undefined") {
            isUnderwater.checked = Math.round(+obj.light_refraction_coefficient * 1000) == 1330;
        }
    }

    Component.onCompleted: {
        root.loadHiddenProfiles();
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
    }
    Timer {
        id: profilesUpdateTimer;
        interval: 1000;
        property bool fromDisk: true;
        onTriggered: controller.load_profiles(fromDisk);
    }
    Connections {
        target: controller;
        function onAll_profiles_loaded(): void {
            if (!lensProfilesListPrepared) { // If it's the first load
                controller.request_profile_ratings();
            }

            lensProfilesListPrepared = true;
            root.refreshCameraBrands();
            root.applyDetectedCamera();

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
            if (!is_main_video || !additional_data.camera_identifier) {
                return;
            }

            const camera_id = additional_data.camera_identifier;
            root.detectedCamera = {
                "brand": camera_id.brand || "",
                "model": camera_id.model || "",
                "lens": camera_id.lens_model || camera_id.lens_info || ""
            };
            root.applyDetectedCamera();
        }
        function onSearch_lens_profile_finished(profiles: list<var>): void {
            root.reviewProfiles = profiles;
            root.reviewIndex = -1;
        }
        function onLens_profile_loaded(json_str: string, filepath: string, checksum: string): void {
            if (json_str) {
                const obj = JSON.parse(json_str);
                if (obj) {
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

                    info.model = lensInfo;

                    root.cropFactor = +obj.crop_factor;

                    if (!root.selected_manually &&
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

                    officialInfo.show = !obj.official && !settings.value("rated-profile-" + checksum, false);
                    officialInfo.canRate = true;
                    officialInfo.thankYou = false;
                    root.profileName = (filepath || obj.name || "").replace(/^.*?[\/\\]([^\/\\]+?)$/, "$1");
                    root.profileOriginalJson = json_str;
                    root.profileChecksum = checksum;
                    root.profilePath = filepath;

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
                    if (mtrx[0][0] > 10000) {
                        window.sync.poseMethod.currentIndex = 1; // Almeida
                    }
                }
                Qt.callLater(controller.recompute_threaded);
            }
        }
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
    function loadHiddenProfiles(): void {
        const stored = settings.value("hiddenLensProfiles", "[]");
        let values = [];
        try {
            values = JSON.parse(stored || "[]");
        } catch (e) {
            values = (stored || "").split(",");
        }
        let hidden = {};
        for (const value of values) {
            if (value) {
                hidden[value] = 1;
            }
        }
        hiddenProfiles = hidden;
    }
    function updateHiddenProfiles(): void {
        settings.setValue("hiddenLensProfiles", JSON.stringify(Object.keys(hiddenProfiles).filter(v => v)));
    }
    function hideProfile(checksum: string, path: string): void {
        const key = checksum || path;
        if (!key) {
            return;
        }
        let hidden = {};
        for (const value of Object.keys(hiddenProfiles)) {
            hidden[value] = 1;
        }
        hidden[key] = 1;
        hiddenProfiles = hidden;
        updateHiddenProfiles();
        root.reviewProfiles = root.reviewProfiles.filter(item => item[2] !== checksum && item[1] !== path);
        if (root.reviewIndex >= root.reviewProfiles.length) {
            root.reviewIndex = root.reviewProfiles.length - 1;
        }
        root.searchProfiles();
    }
    function hideCurrentProfile(): void {
        root.hideProfile(root.profileChecksum, root.profilePath);
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
    function refreshCameraBrands(): void {
        updatingCameraSelectors = true;
        cameraBrand.popup.maxItemWidth = 0;
        cameraModel.popup.maxItemWidth = 0;
        cameraLens.popup.maxItemWidth = 0;
        cameraBrandModel = selectorModel(qsTr("Any brand"), controller.camera_registry_brands());
        cameraBrand.currentIndex = 0;
        cameraModelModel = selectorModel(qsTr("Any model"), []);
        cameraModel.currentIndex = 0;
        cameraLensModel = selectorModel(qsTr("Any lens"), []);
        cameraLens.currentIndex = 0;
        compatibleCameras = [];
        updatingCameraSelectors = false;
    }
    function refreshCameraModels(): void {
        const brand = selectorValue(cameraBrand);
        updatingCameraSelectors = true;
        cameraModel.popup.maxItemWidth = 0;
        cameraLens.popup.maxItemWidth = 0;
        cameraModelModel = selectorModel(qsTr("Any model"), brand? controller.camera_registry_models(brand) : []);
        cameraModel.currentIndex = 0;
        cameraLensModel = selectorModel(qsTr("Any lens"), []);
        cameraLens.currentIndex = 0;
        compatibleCameras = [];
        updatingCameraSelectors = false;
    }
    function refreshCameraLenses(): void {
        const brand = selectorValue(cameraBrand);
        const model = selectorValue(cameraModel);
        updatingCameraSelectors = true;
        cameraLens.popup.maxItemWidth = 0;
        cameraLensModel = selectorModel(qsTr("Any lens"), brand && model? controller.camera_registry_lenses(brand, model) : []);
        cameraLens.currentIndex = 0;
        compatibleCameras = brand && model? controller.camera_registry_compatible_models(brand, model) : [];
        updatingCameraSelectors = false;
    }
    function updateCameraSearch(): void {
        if (!updatingCameraSelectors) {
            searchProfiles();
        }
    }
    function searchProfiles(): void {
        controller.search_lens_profile_for_camera(
            selectorValue(cameraBrand),
            selectorValue(cameraModel),
            selectorValue(cameraLens),
            search.text.trim(),
            Object.keys(root.hiddenProfiles),
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
        if (!detectedCamera.brand || !lensProfilesListPrepared) {
            return;
        }
        updatingCameraSelectors = true;
        cameraBrand.currentIndex = 0;
        cameraModelModel = selectorModel(qsTr("Any model"), []);
        cameraModel.currentIndex = 0;
        cameraLensModel = selectorModel(qsTr("Any lens"), []);
        cameraLens.currentIndex = 0;
        compatibleCameras = [];
        const resolvedCamera = controller.camera_registry_resolve_camera(detectedCamera.brand, detectedCamera.model);
        const brand = resolvedCamera.brand || detectedCamera.brand;
        const model = resolvedCamera.model || detectedCamera.model;
        if (setSelectorValue(cameraBrand, brand)) {
            cameraModelModel = selectorModel(qsTr("Any model"), controller.camera_registry_models(brand));
            if (model && setSelectorValue(cameraModel, model)) {
                cameraLensModel = selectorModel(qsTr("Any lens"), controller.camera_registry_lenses(brand, model));
                compatibleCameras = controller.camera_registry_compatible_models(brand, model);
                if (detectedCamera.lens) {
                    setSelectorValue(cameraLens, detectedCamera.lens);
                }
            }
        }
        updatingCameraSelectors = false;
        updateCameraSearch();
    }
    function loadProfileItem(item: var): void {
        const lensPathOrId = item[1];
        if (lensPathOrId.endsWith(".gyroflow")) {
            window.videoArea.loadGyroflowData(JSON.parse(controller.get_preset_contents(lensPathOrId)), 0);
        } else {
            root.selected_manually = true;
            controller.load_lens_profile(lensPathOrId);
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
        columns: window.isMobileLayout? 1 : 3;
        columnSpacing: 5 * dpiScale;
        rowSpacing: 5 * dpiScale;

        ComboBox {
            id: cameraBrand;
            model: root.cameraBrandModel;
            width: window.isMobileLayout? parent.width : (parent.width - 10 * dpiScale) / 3;
            font.pixelSize: 12 * dpiScale;
            popup.x: root.selectorPopupX(cameraBrand);
            popup.width: root.selectorPopupWidth(cameraBrand);
            popup.height: Math.min(popup.implicitHeight, 8 * itemHeight + 4 * dpiScale);
            onCurrentIndexChanged: {
                if (!root.updatingCameraSelectors) {
                    root.refreshCameraModels();
                    root.updateCameraSearch();
                }
            }
        }
        ComboBox {
            id: cameraModel;
            model: root.cameraModelModel;
            width: window.isMobileLayout? parent.width : (parent.width - 10 * dpiScale) / 3;
            font.pixelSize: 12 * dpiScale;
            popup.x: root.selectorPopupX(cameraModel);
            popup.width: root.selectorPopupWidth(cameraModel);
            popup.height: Math.min(popup.implicitHeight, 8 * itemHeight + 4 * dpiScale);
            onCurrentIndexChanged: {
                if (!root.updatingCameraSelectors) {
                    root.refreshCameraLenses();
                    root.updateCameraSearch();
                }
            }
        }
        ComboBox {
            id: cameraLens;
            model: root.cameraLensModel;
            width: window.isMobileLayout? parent.width : (parent.width - 10 * dpiScale) / 3;
            font.pixelSize: 12 * dpiScale;
            popup.x: root.selectorPopupX(cameraLens);
            popup.width: root.selectorPopupWidth(cameraLens);
            popup.height: Math.min(popup.implicitHeight, 8 * itemHeight + 4 * dpiScale);
            onCurrentIndexChanged: if (!root.updatingCameraSelectors) root.updateCameraSearch();
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
            onClicked: root.hideCurrentProfile();
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
                    root.hideCurrentProfile();
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
    }

    AdvancedSection {
        btn.text: qsTr("Advanced");
        visible: Object.keys(info.model).length > 0

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
        LinkButton {
            anchors.horizontalCenter: parent.horizontalCenter;
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
