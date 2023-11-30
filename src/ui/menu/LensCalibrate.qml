// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Dialogs
import Qt.labs.settings

import "../components/"

MenuItem {
    id: calib;
    text: qsTr("Calibration");
    iconName: "lens";
    innerItem.enabled: controller && !controller.calib_in_progress;
    loader: false;//controller && controller.calib_in_progress;
    objectName: "lenscalib";

    property alias autoCalibBtn: autoCalibBtn;
    property alias uploadProfile: uploadProfile;
    property alias noMarker: noMarker.checked;
    property alias previewResolution: previewResolution.currentIndex;
    property alias infoList: infoList;
    property alias maxSharpness: maxSharpness;
    property var calibrationInfo: ({});

    property int videoWidth: 0;
    property int videoHeight: 0;
    property real fps: 0;

    function setVideoSize(w: int, h: int) {
        videoWidth = w;
        videoHeight = h;
        sizeTimer.start();
    }

    Timer {
        id: sizeTimer;
        interval: 1;
        onTriggered: {
            let w = videoWidth;
            let h = videoHeight;
            let videoRatio = w / h;
            if (Math.round(videoRatio * 100) == 133) { // 4:3
                // Default to 16:9 output
                h = Math.round(w / (16 / 9));
                if ((h % 2) != 0) h--;
            }

            calib.calibrationInfo.fps = fps;
            list.updateEntryWithTrigger("Default output size", w + "x" + h);
            fovSlider.value = 2;
            xStretch.valueChanged();
            yStretch.valueChanged();
        }
    }
    function resetMetadata() {
        calib.calibrationInfo = {
            "calibrated_by": calib.calibrationInfo.calibrated_by || settings.value("calibratedBy", "") || controller.get_username(),
            "output_dimension": { "w": 0, "h": 0 }
        };
    }
    function updateTable() {
        const fields = {
            "camera_brand":     QT_TRANSLATE_NOOP("TableList", "Camera brand"),
            "camera_model":     QT_TRANSLATE_NOOP("TableList", "Camera model"),
            "lens_model":       QT_TRANSLATE_NOOP("TableList", "Lens model"),
            "camera_setting":   QT_TRANSLATE_NOOP("TableList", "Camera setting"),
            "note":             QT_TRANSLATE_NOOP("TableList", "Additional info"),
            "output_dimension": QT_TRANSLATE_NOOP("TableList", "Default output size"),
            "identifier":       QT_TRANSLATE_NOOP("TableList", "Identifier"),
            "calibrated_by":    QT_TRANSLATE_NOOP("TableList", "Calibrated by")
        };
        let model = {};
        for (const x in fields) {
            let v = calib.calibrationInfo[x];
            if (v && x == "output_dimension") {
                v = v.w + "x" + v.h;
            }
            model[fields[x]] = v || "---";
        }
        list.model = model;
    }
    Component.onCompleted: {
        calib.resetMetadata();
        calib.updateTable();
    }
    Connections {
        target: controller;
        function onTelemetry_loaded(is_main_video: bool, filename: string, camera: string, additional_data: var) {
            shutter.value = Math.abs(additional_data.frame_readout_time);
            shutterCb.checked = Math.abs(additional_data.frame_readout_time) > 0;
            bottomToTop.checked = additional_data.frame_readout_time < 0;

            calib.resetMetadata();
            if (additional_data.camera_identifier) {
                const camera_id = additional_data.camera_identifier;
                if (camera_id) {
                    if (camera_id.brand)      { calib.calibrationInfo.camera_brand = camera_id.brand; }
                    if (camera_id.model)      { calib.calibrationInfo.camera_model = camera_id.model; }
                    if (camera_id.lens_model) { calib.calibrationInfo.lens_model   = calib.calibrationInfo.lens_model? calib.calibrationInfo.lens_model + " " + camera_id.lens_model : camera_id.lens_model; }
                    if (camera_id.lens_info)  { calib.calibrationInfo.lens_model   = calib.calibrationInfo.lens_model? calib.calibrationInfo.lens_model + " " + camera_id.lens_info  : camera_id.lens_info;  }
                    if (camera_id.camera_setting) { calib.calibrationInfo.camera_setting = camera_id.camera_setting; }
                    if (camera_id.additional) { calib.calibrationInfo.note         = camera_id.additional; }
                    if (camera_id.identifier) { calib.calibrationInfo.identifier   = camera_id.identifier; }
                    if (camera_id.fps)        { calib.calibrationInfo.fps          = camera_id.fps / 1000.0; }
                    if (+camera_id.focal_length > 0) { flcb.checked = true; fl.value = +camera_id.focal_length; }

                    if (camera_id.brand === "GoPro" && camera_id.lens_info === "Super") digitalLens.currentIndex = 1;
                    if (camera_id.brand === "GoPro" && camera_id.lens_info === "Hyper") digitalLens.currentIndex = 2;

                    // RED KOMODO is global shutter
                    gs.checked = camera_id.model.startsWith("KOMODO");
                }
            }
            if (+additional_data.horizontal_stretch > 0.01) xStretch.value = +additional_data.horizontal_stretch;
            if (+additional_data.vertical_stretch   > 0.01) yStretch.value = +additional_data.vertical_stretch;
            calib.updateTable();
            sizeTimer.start();
        }
        function onRolling_shutter_estimated(rolling_shutter: real) {
            shutter.value = Math.abs(rolling_shutter);
            shutterCb.checked = Math.abs(rolling_shutter) > 0;
            bottomToTop.checked = rolling_shutter < 0;
        }
    }

    Settings {
        id: settings;
        property alias calib_maxPoints: maxPoints.value;
        property alias calib_everyNthFrame: everyNthFrame.value;
        property alias calib_iterations: iterations.value;
        property alias calib_maxSharpness: maxSharpness.value;
    }

    FileDialog {
        id: fileDialog;
        fileMode: FileDialog.SaveFile;
        defaultSuffix: "json";

        title: qsTr("Export lens profile");
        nameFilters: Qt.platform.os == "android"? undefined : [qsTr("Lens profiles") + " (*.json)"];
        type: "output-preset";
        onAccepted: {
            if (uploadProfile.checked) {
                messageBox(Modal.Info, qsTr("By uploading your lens profile to the database, you agree to publish and distribute it with Gyroflow under GPLv3 terms.\nDo you want to submit your profile?"), [
                    { text: qsTr("Yes"), accent: true, clicked: () => controller.export_lens_profile(selectedFile, calib.calibrationInfo, true) },
                    { text: qsTr("No"),                clicked: () => controller.export_lens_profile(selectedFile, calib.calibrationInfo, false) }
                ]);
            } else {
                controller.export_lens_profile(selectedFile, calib.calibrationInfo, uploadProfile.checked);
            }
        }
    }

    InfoMessageSmall {
        show: infoList.rms > 5 && infoList.rms < 100;
        text: qsTr("For a good lens calibration, this value should be less than 5, ideally less than 1.");
    }
    TableList {
        id: infoList;
        columnSpacing: 10 * dpiScale;
        property real rms: 0;
        onModelChanged: {
            Qt.callLater(() => {
                if (infoList.col2.children.length > 0 && infoList.col2.children[0].children.length > 0) {
                    infoList.col2.children[0].children[0].color = rms == 0? styleTextColor : rms < 1? "#1ae921" : rms < 5? "#f6a10c" : "#f41717";
                }
            });
        }
    }

    Button {
        id: autoCalibBtn;
        text: qsTr("Auto calibrate");
        enabled: calibrator_window.videoArea.vid.loaded;
        iconName: "spinner"
        anchors.horizontalCenter: parent.horizontalCenter;
        onClicked: {
            controller.start_autocalibrate(maxPoints.value, everyNthFrame.value, iterations.value, maxSharpness.value, -1, noMarker.checked);
        }
    }

    Label {
        position: Label.LeftPosition;
        text: qsTr("Max calibration points");

        NumberField {
            id: maxPoints;
            width: parent.width;
            height: 25 * dpiScale;
            value: 15;
            from: 1;
        }
    }

    TableList {
        id: list;
        columnSpacing: 10 * dpiScale;
        editableFields: ({
            "Camera brand": {
                "type": "text",
                "width": 120,
                "value": function() { return calib.calibrationInfo.camera_brand || ""; },
                "onChange": function(value) { calib.calibrationInfo.camera_brand = value; list.updateEntry("Camera brand", value); }
            },
            "Camera model": {
                "type": "text",
                "width": 120,
                "value": function() { return calib.calibrationInfo.camera_model || ""; },
                "onChange": function(value) { calib.calibrationInfo.camera_model = value; list.updateEntry("Camera model", value);  }
            },
            "Lens model": {
                "type": "text",
                "width": 120,
                "value": function() { return calib.calibrationInfo.lens_model || ""; },
                "onChange": function(value) { calib.calibrationInfo.lens_model = value; list.updateEntry("Lens model", value); }
            },
            "Camera setting": {
                "type": "text",
                "width": 120,
                "value": function() { return calib.calibrationInfo.camera_setting || ""; },
                "onChange": function(value) { calib.calibrationInfo.camera_setting = value; list.updateEntry("Camera setting", value); }
            },
            "Additional info": {
                "type": "text",
                "width": 120,
                "value": function() { return calib.calibrationInfo.note || ""; },
                "onChange": function(value) { calib.calibrationInfo.note = value; list.updateEntry("Additional info", value);  }
            },
            "Default output size": {
                "type": "text",
                "width": 120,
                "value": function() { return calib.calibrationInfo.output_dimension? (calib.calibrationInfo.output_dimension.w + "x" + calib.calibrationInfo.output_dimension.h) : ""; },
                "onChange": function(value) {
                    if (/^[0-9]{1,5}x[0-9]{1,5}$/.test(value)) {
                        list.updateEntry("Default output size", value);

                        const parts = value.split('x');
                        const ow = +parts[0], oh = +parts[1];

                        calib.calibrationInfo.output_dimension = { "w": ow, "h": oh };
                        calibrator_window.videoArea.outWidth = ow;
                        calibrator_window.videoArea.outHeight = oh;
                        controller.set_output_size(ow, oh);
                    } else {
                        window.messageBox(Modal.Error, qsTr("Invalid format"), [ { "text": qsTr("Ok") } ], calibrator_window.contentItem);
                    }
                }
            },
            "Calibrated by": {
                "type": "text",
                "width": 120,
                "value": function() { return calib.calibrationInfo.calibrated_by || ""; },
                "onChange": function(value) { calib.calibrationInfo.calibrated_by = value; list.updateEntry("Calibrated by", value); settings.setValue("calibratedBy", value); }
            }
        });
    }
    CheckBoxWithContent {
        id: shutterCb;
        text: qsTr("Rolling shutter correction");
        cb.onCheckedChanged: {
            const v = cb.checked? (bottomToTop.checked? -shutter.value : shutter.value) : 0.0;
            controller.frame_readout_time = v;
            calib.calibrationInfo.frame_readout_time = v;
        }

        Label {
            text: qsTr("Frame readout time");
            SliderWithField {
                id: shutter;
                defaultValue: 0.0;
                to: 1000 / Math.max(1, calibrator_window.videoArea.vid.frameRate);
                width: parent.width;
                unit: qsTr("ms");
                precision: 2;
                onValueChanged: {
                    const v = bottomToTop.checked? -value : value;
                    controller.frame_readout_time = v;
                    calib.calibrationInfo.frame_readout_time = v;
                }
            }
            CheckBox {
                id: bottomToTop;
                anchors.right: parent.right;
                anchors.top: parent.top;
                anchors.topMargin: -30 * dpiScale;
                anchors.rightMargin: -10 * dpiScale;
                contentItem.visible: false;
                scale: 0.7;
                tooltip: qsTr("Bottom to top")
                onCheckedChanged: {
                    const v = bottomToTop.checked? -shutter.value : shutter.value;
                    controller.frame_readout_time = v;
                    calib.calibrationInfo.frame_readout_time = v;
                }
            }
        }
    }
    Item { width: 1; height: 1; }
    Button {
        text: qsTr("Export lens profile");
        accent: true;
        iconName: "save"
        enabled: infoList.rms > 0 && infoList.rms < 100 && calibrator_window.videoArea.vid.loaded;
        anchors.horizontalCenter: parent.horizontalCenter;
        onClicked: {
            list.commitAll();
            fileDialog.selectedFile = controller.export_lens_profile_filename(calib.calibrationInfo);
            fileDialog.open2();
        }
    }
    CheckBox {
        id: uploadProfile;
        text: qsTr("Upload lens profile to the database");
        checked: true;
    }
    AdvancedSection {
        Label {
            position: Label.LeftPosition;
            text: qsTr("FOV");
            SliderWithField {
                id: fovSlider;
                from: 0.1;
                to: 3;
                value: 1.0;
                defaultValue: 1.0;
                width: parent.width;
                onValueChanged: controller.fov = value;
            }
        }
        Label {
            position: Label.LeftPosition;
            text: qsTr("Analyze every n-th frame");

            NumberField {
                id: everyNthFrame;
                width: parent.width;
                height: 25 * dpiScale;
                value: 10;
                from: 1;
            }
        }
        Label {
            position: Label.LeftPosition;
            text: qsTr("Sharpness limit");

            NumberField {
                id: maxSharpness;
                width: parent.width;
                height: 25 * dpiScale;
                precision: 2;
                value: 8;
                from: 1;
                unit: qsTr("px");

                // tooltip: qsTr("Chessboard sharpness for determining the quality");
            }
        }

        Label {
            position: Label.LeftPosition;
            text: qsTr("Digital lens");

            ComboBox {
                id: digitalLens;
                property var lenses: [
                    ["None", ""],
                    ["GoPro Superview", "gopro_superview"],
                    ["GoPro Hyperview", "gopro_hyperview"]/*,
                    ["Stretch", "digital_stretch", [
                        { "label": QT_TR_NOOP("X"), "from": 10, "to": 200, "scale": 100, "value": 100, "unit": "%" },
                        { "label": QT_TR_NOOP("Y"), "from": 10, "to": 200, "scale": 100, "value": 100, "unit": "%" }
                    ]]*/
                ];
                model: lenses.map(x => x[0]);
                font.pixelSize: 12 * dpiScale;
                width: parent.width;
                currentIndex: 0;
                onCurrentIndexChanged: {
                    controller.set_digital_lens_name(lenses[currentIndex][1]);
                    if (lenses[currentIndex][2]) {
                        digitalParams.model = lenses[currentIndex][2];
                        digitalParamsCol.visible = true;
                    } else {
                        digitalParams.model = [];
                        digitalParamsCol.visible = false;
                    }
                }
            }
        }
        Column {
            width: parent.width;
            spacing: 5 * dpiScale;
            id: digitalParamsCol;
            visible: false;
            Repeater {
                id: digitalParams;
                Label {
                    position: Label.LeftPosition;
                    text: qsTr(modelData.label);
                    SliderWithField {
                        from: modelData.from;
                        to: modelData.to;
                        value: modelData.value / modelData.scale;
                        defaultValue: modelData.value;
                        width: parent.width;
                        precision: 2;
                        unit: modelData.unit;
                        scaler: modelData.scale;
                        onValueChanged: {
                            controller.set_digital_lens_param(index, value);
                            if (!calib.calibrationInfo.digital_lens_params)
                                calib.calibrationInfo.digital_lens_params = [];
                            calib.calibrationInfo.digital_lens_params[index] = value;
                        }
                    }
                }
            }
        }

        Label {
            position: Label.TopPosition;
            text: qsTr("Input horizontal stretch");
            SliderWithField {
                id: xStretch;
                from: 0.1;
                to: 2;
                value: 1.0;
                defaultValue: 1.0;
                width: parent.width;
                onValueChanged: {
                    controller.input_horizontal_stretch = value;
                    calib.calibrationInfo.input_horizontal_stretch = value;
                    updateResolutionTimer.start();
                }
            }
        }
        Label {
            position: Label.TopPosition;
            text: qsTr("Input vertical stretch");
            SliderWithField {
                id: yStretch;
                from: 0.1;
                to: 2;
                value: 1.0;
                defaultValue: 1.0;
                width: parent.width;
                onValueChanged: {
                    controller.input_vertical_stretch = value;
                    calib.calibrationInfo.input_vertical_stretch = value;
                    updateResolutionTimer.start();
                }
            }
        }
        Timer {
            id: updateResolutionTimer;
            interval: 1500;
            onTriggered: {
                let w = Math.round(calib.videoWidth  * (xStretch.value || 1));
                let h = Math.round(calib.videoHeight * (yStretch.value || 1));
                if ((xStretch.value || 1) != 1 && (w % 2) != 0) w--;
                if ((yStretch.value || 1) != 1 && (h % 2) != 0) h--;
                if (calib.calibrationInfo.output_dimension.w != w || calib.calibrationInfo.output_dimension.h != h) {
                    messageBox(Modal.Info, qsTr("Do you want to update the output resolution to %1?").arg("<b>" + w + "x" + h + "</b>"), [
                        { text: qsTr("Yes"), accent: true, clicked: () => {
                            list.updateEntryWithTrigger("Default output size", w + "x" + h);
                        } },
                        { text: qsTr("No"), }
                    ], null, undefined, "update-resolution");
                }
            }
        }
        Label {
            position: Label.LeftPosition;
            text: qsTr("Iterations");

            NumberField {
                id: iterations;
                width: parent.width;
                height: 25 * dpiScale;
                value: 500;
                from: 1;
            }
        }
        CheckBoxWithContent {
            id: lpfcb;
            text: qsTr("Low pass filter");
            onCheckedChanged: {
                const v = checked? lpf.value : 0;
                controller.set_imu_lpf(v);
                Qt.callLater(controller.recompute_gyro);
                calib.calibrationInfo.gyro_lpf = v;
            }
            NumberField {
                id: lpf;
                unit: qsTr("Hz");
                precision: 2;
                value: 50;
                from: 0;
                width: parent.width;
                onValueChanged: {
                    const v = lpfcb.checked? value : 0;
                    controller.set_imu_lpf(v);
                    Qt.callLater(controller.recompute_gyro);
                    calib.calibrationInfo.gyro_lpf = v;
                }
            }
        }
        CheckBoxWithContent {
            id: flcb;
            text: qsTr("Focal length");
            Label {
                text: qsTr("Lens native focal length");
                position: Label.LeftPosition;
                NumberField {
                    id: fl;
                    unit: qsTr("mm");
                    precision: 2;
                    value: 0;
                    from: 0;
                    width: parent.width;
                    onValueChanged: calib.calibrationInfo.focal_length = flcb.checked? value : null;
                }
            }
            Label {
                text: qsTr("Crop factor");
                position: Label.LeftPosition;
                NumberField {
                    id: crop;
                    unit: qsTr("x");
                    precision: 2;
                    value: 1;
                    from: 0;
                    to: 10;
                    width: parent.width;
                    onValueChanged: calib.calibrationInfo.crop_factor = flcb.checked? value : null;
                }
            }
        }
        Label {
            position: Label.LeftPosition;
            text: qsTr("Preview resolution");

            ComboBox {
                id: previewResolution;
                model: [QT_TRANSLATE_NOOP("Popup", "Full"), "4k", "1080p", "720p", "480p"];
                font.pixelSize: 12 * dpiScale;
                width: parent.width;
                currentIndex: 0;
                onCurrentIndexChanged: {
                    let target_height = -1; // Full
                    switch (currentIndex) {
                        case 0: calibrator_window.videoArea.vid.setProperty("scale", ""); break;
                        case 1: target_height = 2160; calibrator_window.videoArea.vid.setProperty("scale", "3840x2160"); break;
                        case 2: target_height = 1080; calibrator_window.videoArea.vid.setProperty("scale", "1920x1080"); break;
                        case 3: target_height = 720;  calibrator_window.videoArea.vid.setProperty("scale", "1280x720");  break;
                        case 4: target_height = 480;  calibrator_window.videoArea.vid.setProperty("scale", "640x480");   break;
                    }
                    controller.set_preview_resolution(target_height, calibrator_window.videoArea.vid);
                }
            }
        }
        Label {
            position: Label.LeftPosition;
            text: qsTr("Processing resolution");
            ComboBox {
                id: processingResolution;
                model: [QT_TRANSLATE_NOOP("Popup", "Full"), "4k", "1080p", "720p", "480p"];
                font.pixelSize: 12 * dpiScale;
                width: parent.width;
                currentIndex: 1;
                Component.onCompleted: currentIndexChanged();
                onCurrentIndexChanged: {
                    let target_height = -1; // Full
                    switch (currentIndex) {
                        case 1: target_height = 2160; break;
                        case 2: target_height = 1080; break;
                        case 3: target_height = 720;  break;
                        case 4: target_height = 480;  break;
                    }

                    controller.set_processing_resolution(target_height);
                }
            }
        }
        InfoMessageSmall {
            show: processingResolution.currentIndex > 1;
            text: qsTr("Lens calibration should be processed at full resolution or at least at 4k. Change this setting only if you know what you're doing.");
        }
        CheckBoxWithContent {
            id: rLimitCb;
            text: qsTr("Radial distortion limit");
            cb.onCheckedChanged: {
                controller.set_lens_param("r_limit", checked? rLimit.value : 0);
            }

            SliderWithField {
                id: rLimit;
                defaultValue: 0;
                width: parent.width;
                precision: 2;
                from: 0;
                to: 10;
                onValueChanged: controller.set_lens_param("r_limit", rLimitCb.checked? value : 0);
            }
        }
        CheckBox {
            text: qsTr("Lens is asymmetrical");
            checked: false;
            width: parent.width;
            onCheckedChanged: controller.lens_is_asymmetrical = checked;
        }
        CheckBox {
            id: gs;
            text: qsTr("Sensor is global shutter");
            checked: false;
            width: parent.width;
            onCheckedChanged: calib.calibrationInfo.global_shutter = checked;
        }
        CheckBox {
            id: noMarker;
            text: qsTr("Plain chessboard pattern (previous version without dots in the middle)");
            checked: false;
            width: parent.width;
            Component.onCompleted: contentItem.wrapMode = Text.WordWrap;
        }
    }
}
