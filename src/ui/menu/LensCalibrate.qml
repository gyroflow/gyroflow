// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Dialogs
import Qt.labs.settings

import "../components/"

MenuItem {
    id: calib;
    text: qsTr("Calibration");
    icon: "lens";
    innerItem.enabled: !controller.calib_in_progress;
    loader: controller.calib_in_progress;

    property alias rms: rms.value;
    property alias autoCalibBtn: autoCalibBtn;
    property alias uploadProfile: uploadProfile;
    property var calibrationInfo: ({});

    property int videoWidth: 0;
    property int videoHeight: 0;
    property real fps: 0;
    onVideoWidthChanged: {
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

            calib.calibrationInfo.output_dimension = { "w": w, "h": h };
            calib.calibrationInfo.fps = fps;
            list.updateEntry("Default output size", w + "x" + h);
            calibrator_window.videoArea.outWidth = w;
            calibrator_window.videoArea.outHeight = h;
            controller.set_output_size(w, h);
        }
    }
    function resetMetadata() {
        calib.calibrationInfo = {
            "calibrated_by": calib.calibrationInfo.calibrated_by || controller.get_username(),
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
        function onTelemetry_loaded(is_main_video, filename, camera, imu_orientation, contains_gyro, contains_quats, frame_readout_time, camera_id_json) {
            shutter.value = Math.abs(frame_readout_time);
            shutterCb.checked = Math.abs(frame_readout_time) > 0;
            bottomToTop.checked = frame_readout_time < 0;

            calib.resetMetadata();
            if (camera_id_json) {
                const camera_id = JSON.parse(camera_id_json);
                if (camera_id) {
                    if (camera_id.brand)      { calib.calibrationInfo.camera_brand = camera_id.brand; }
                    if (camera_id.model)      { calib.calibrationInfo.camera_model = camera_id.model; }
                    if (camera_id.lens_model) { calib.calibrationInfo.lens_model   = calib.calibrationInfo.lens_model? calib.calibrationInfo.lens_model + " " + camera_id.lens_model : camera_id.lens_model; }
                    if (camera_id.lens_info)  { calib.calibrationInfo.lens_model   = calib.calibrationInfo.lens_model? calib.calibrationInfo.lens_model + " " + camera_id.lens_info  : camera_id.lens_info;  }
                    if (camera_id.additional) { calib.calibrationInfo.note         = camera_id.additional; }
                    if (camera_id.identifier) { calib.calibrationInfo.identifier   = camera_id.identifier; }
                    if (camera_id.fps)        { calib.calibrationInfo.fps          = camera_id.fps / 1000.0; }
                }
            }
            calib.updateTable();
            sizeTimer.start();
        }
        function onRolling_shutter_estimated(rolling_shutter) {
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
        onAccepted: {
            if (uploadProfile.checked) {
                messageBox(Modal.Info, qsTr("By uploading your lens profile to the database, you agree to publish and distribute it with Gyroflow under GPLv3 terms.\nDo you want to submit your profile?"), [
                    { text: qsTr("Yes"), accent: true, clicked: () => controller.export_lens_profile(fileDialog.selectedFile, calib.calibrationInfo, true) },
                    { text: qsTr("No"),                clicked: () => controller.export_lens_profile(fileDialog.selectedFile, calib.calibrationInfo, false) }
                ]);
            } else {
                controller.export_lens_profile(fileDialog.selectedFile, calib.calibrationInfo, uploadProfile.checked);
            }
        }
    }

    Item {
        width: parent.width;
        height: rmsLabel.height;
        Label {
            id: rmsLabel;
            position: Label.Left;
            text: qsTr("Reprojection error") + ":";

            BasicText {
                id: rms;
                property real value: 0;
                font.bold: true;
                text: value == 0? "---" : value.toLocaleString(Qt.locale(), "f", 5)
                color: value == 0? styleTextColor : value < 1? "#1ae921" : value < 5? "#f6a10c" : "#f41717";
                anchors.verticalCenter: parent.verticalCenter;
            }
        }
        MouseArea { id: rmsMa; anchors.fill: parent; hoverEnabled: true; }
        ToolTip { visible: rmsMa.containsMouse; text: qsTr("For a good lens calibration, this value should be less than 5, ideally less than 1.") }
    }
    Button {
        id: autoCalibBtn;
        text: qsTr("Auto calibrate");
        enabled: calibrator_window.videoArea.vid.loaded;
        icon.name: "spinner"
        anchors.horizontalCenter: parent.horizontalCenter;
        onClicked: {
            controller.start_autocalibrate(maxPoints.value, everyNthFrame.value, iterations.value, maxSharpness.value, -1);
        }
    }

    Label {
        position: Label.Left;
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
        spacing: 10 * dpiScale;
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
                "onChange": function(value) { calib.calibrationInfo.calibrated_by = value; list.updateEntry("Calibrated by", value);  }
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
        icon.name: "save"
        enabled: rms.value > 0 && rms.value < 100 && calibrator_window.videoArea.vid.loaded;
        anchors.horizontalCenter: parent.horizontalCenter;
        onClicked: {
            list.commitAll();
            fileDialog.currentFile = controller.export_lens_profile_filename(calib.calibrationInfo);
            fileDialog.open();
        }
    }
    CheckBox {
        id: uploadProfile;
        text: qsTr("Upload lens profile to the database");
        checked: true;
    }
    AdvancedSection {
        Label {
            position: Label.Left;
            text: qsTr("FOV");
            SliderWithField {
                from: 0.1;
                to: 3;
                value: 1.0;
                defaultValue: 1.0;
                width: parent.width;
                onValueChanged: controller.fov = value;
            }
        }
        Label {
            position: Label.Left;
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
            position: Label.Left;
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
            position: Label.Left;
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
                    calib.calibrationInfo.gyro_lpf = v;
                }
            }
        }
        Label {
            position: Label.Left;
            text: qsTr("Preview resolution");

            ComboBox {
                model: [QT_TRANSLATE_NOOP("Popup", "Full"), "1080p", "720p", "480p"];
                font.pixelSize: 12 * dpiScale;
                width: parent.width;
                currentIndex: 2;
                onCurrentIndexChanged: {
                    let target_height = -1; // Full
                    switch (currentIndex) {
                        case 1: target_height = 1080; break;
                        case 2: target_height = 720; break;
                        case 3: target_height = 480; break;
                    }

                    controller.set_preview_resolution(target_height, calibrator_window.videoArea.vid);
                }
            }
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
    }
}
