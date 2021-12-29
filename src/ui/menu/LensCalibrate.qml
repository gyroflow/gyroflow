import QtQuick 2.15
import Qt.labs.settings 1.0

import "../components/"

MenuItem {
    id: calib;
    text: qsTr("Calibration");
    icon: "lens";
    innerItem.enabled: calibrator_window.videoArea.vid.loaded && !controller.calib_in_progress;
    loader: controller.calib_in_progress;

    Settings {
        id: settings;
        property alias calib_maxPoints: maxPoints.value;
        property alias calib_everyNthFrame: everyNthFrame.value;
        property alias calib_iterations: iterations.value;
        property alias calib_maxSharpness: maxSharpness.value;
    }

    property alias rms: rms.value;

    property var calibrationInfo: ({});

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
            controller.start_autocalibrate(maxPoints.value, everyNthFrame.value, iterations.value, maxSharpness.value);
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
                "onChange": function(value) { calib.calibrationInfo.output_size = value; list.updateEntry("Default output size", value);  }
            },
            "Identifier": {
                "type": "text",
                "width": 120,
                "value": function() { return calib.calibrationInfo.identifier || ""; },
                "onChange": function(value) { calib.calibrationInfo.identifier = value; list.updateEntry("Identifier", value);  }
            },
            "Calibrated by": {
                "type": "text",
                "width": 120,
                "value": function() { return calib.calibrationInfo.calibrated_by || ""; },
                "onChange": function(value) { calib.calibrationInfo.calibrated_by = value; list.updateEntry("Calibrated by", value);  }
            }
        });
    }
    Item { width: 1; height: 1; }
    Button {
        text: qsTr("Export lens profile");
        accent: true;
        icon.name: "save"
        anchors.horizontalCenter: parent.horizontalCenter;
        onClicked: {
            // TODO
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
            Slider {
                from: 0.1;
                to: 3;
                value: 1.0;
                width: parent.width;
                onValueChanged: { controller.fov = value; controller.recompute_calib_undistortion(); }
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
                value: 3;
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
        CheckBoxWithContent {
            id: shutterCb;
            text: qsTr("Rolling shutter correction");
            cb.onCheckedChanged: {
                controller.frame_readout_time = cb.checked? (bottomToTop.checked? -shutter.value : shutter.value) : 0.0;
                controller.recompute_calib_undistortion();
            }

            Label {
                text: qsTr("Frame readout time");
                SliderWithField {
                    id: shutter;
                    to: 1000 / Math.max(1, calibrator_window.videoArea.vid.frameRate);
                    width: parent.width;
                    unit: qsTr("ms");
                    precision: 2;
                    onValueChanged: { controller.frame_readout_time = bottomToTop.checked? -value : value; controller.recompute_calib_undistortion(); }
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
                    onCheckedChanged: { controller.frame_readout_time = bottomToTop.checked? -shutter.value : shutter.value; controller.recompute_calib_undistortion(); }
                }
            }
        }
    }
}
