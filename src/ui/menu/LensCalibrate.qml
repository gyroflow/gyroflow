import QtQuick 2.15
import QtQuick.Dialogs
import Qt.labs.settings 1.0

import "../components/"

MenuItem {
    id: calib;
    text: qsTr("Calibration");
    icon: "lens";
    innerItem.enabled: calibrator_window.videoArea.vid.loaded && !controller.calib_in_progress;
    loader: controller.calib_in_progress;

    property alias rms: rms.value;
    property var calibrationInfo: ({});

    property int videoWidth: 0;
    property int videoHeight: 0;
    onVideoWidthChanged: {
        Qt.callLater(function() {
            calib.calibrationInfo.output_size = videoWidth + "x" + videoHeight;
            list.updateEntry("Default output size", calib.calibrationInfo.output_size);
        });
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
            controller.export_lens_profile(fileDialog.selectedFile, calib.calibrationInfo, uploadProfile.checked);
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
        text: qsTr("Auto calibrate");
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
            value: 10;
            from: 1;
        }
    }
    Component.onCompleted: {
        const fields = [
            QT_TRANSLATE_NOOP("TableList", "Camera brand"),
            QT_TRANSLATE_NOOP("TableList", "Camera model"),
            QT_TRANSLATE_NOOP("TableList", "Lens model"),
            QT_TRANSLATE_NOOP("TableList", "Camera setting"),
            QT_TRANSLATE_NOOP("TableList", "Additional info"),
            QT_TRANSLATE_NOOP("TableList", "Default output size"),
            QT_TRANSLATE_NOOP("TableList", "Identifier"),
            QT_TRANSLATE_NOOP("TableList", "Calibrated by")
        ];
        let model = {};
        for (const x of fields) model[x] = "---";
        list.model = model;

        calib.calibrationInfo.calibrated_by = controller.get_username();
        list.updateEntry("Calibrated by", calib.calibrationInfo.calibrated_by);
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
                "value": function() { return calib.calibrationInfo.additional_info || ""; },
                "onChange": function(value) { calib.calibrationInfo.additional_info = value; list.updateEntry("Additional info", value);  }
            },
            "Default output size": {
                "type": "text",
                "width": 120,
                "value": function() { return calib.calibrationInfo.output_size || ""; },
                "onChange": function(value) {
                    if (/^[0-9]{1,5}x[0-9]{1,5}$/.test(value)) {
                        calib.calibrationInfo.output_size = value;
                        list.updateEntry("Default output size", value);
                        
                        const parts = value.split('x');
                        const ow = parts[0], oh = parts[1];
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
        cb.onCheckedChanged: controller.frame_readout_time = cb.checked? (bottomToTop.checked? -shutter.value : shutter.value) : 0.0;

        Label {
            text: qsTr("Frame readout time");
            SliderWithField {
                id: shutter;
                to: 1000 / Math.max(1, calibrator_window.videoArea.vid.frameRate);
                width: parent.width;
                unit: qsTr("ms");
                precision: 2;
                onValueChanged: controller.frame_readout_time = bottomToTop.checked? -value : value;
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
                onCheckedChanged: controller.frame_readout_time = bottomToTop.checked? -shutter.value : shutter.value;
            }
        }
    }
    Item { width: 1; height: 1; }
    Button {
        text: qsTr("Export lens profile");
        accent: true;
        icon.name: "save"
        anchors.horizontalCenter: parent.horizontalCenter;
        onClicked: fileDialog.open();
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
                value: 5;
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
                value: 1000;
                from: 1;
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
    }
}
