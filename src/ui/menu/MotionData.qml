// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Dialogs

import "../components/"

MenuItem {
    id: root;
    text: qsTr("Motion data");
    icon: "chart";

    property alias hasQuaternions: integrator.hasQuaternions;
    property alias integrationMethod: integrator.currentIndex;
    property string filename: "";

    FileDialog {
        id: fileDialog;
        property var extensions: [
            "csv", "txt", "bbl", "bfl", "mp4", "mov", "mxf", "gyroflow", "insv", "360", 
            "CSV", "TXT", "BBL", "BFL", "MP4", "MOV", "MXF", "GYROFLOW", "INSV"
        ];

        title: qsTr("Choose a motion data file")
        nameFilters: Qt.platform.os == "android"? undefined : [qsTr("Motion data files") + " (*." + extensions.join(" *.") + ")"];
        onAccepted: loadFile(selectedFile);
    }
    function loadFile(url) {
        if (Qt.platform.os == "android") {
            url = Qt.resolvedUrl("file://" + controller.resolve_android_url(url.toString()));
        }
        controller.load_telemetry(url, false, window.videoArea.vid, window.videoArea.timeline.getChart());
    }

    Connections {
        target: controller;
        function onTelemetry_loaded(is_main_video, filename, camera, imu_orientation, contains_gyro, contains_quats, frame_readout_time, camera_id_json) {
            root.filename = filename || "";
            info.updateEntry("File name", filename || "---");
            info.updateEntry("Detected format", camera || "---");
            orientation.text = imu_orientation;
            integrator.hasQuaternions = contains_quats;
        }
    }

    Button {
        text: qsTr("Open file");
        icon.name: "file-empty"
        anchors.horizontalCenter: parent.horizontalCenter;
        onClicked: fileDialog.open();
    }
    TableList {
        id: info;
            
        Component.onCompleted: {
            QT_TRANSLATE_NOOP("TableList", "File name"),
            QT_TRANSLATE_NOOP("TableList", "Detected format")
        }

        model: ({
            "File name": "---",
            "Detected format": "---"
        })
    }
    CheckBoxWithContent {
        id: lpfcb;
        text: qsTr("Low pass filter");
        onCheckedChanged: controller.set_imu_lpf(checked? lpf.value : 0);

        NumberField {
            id: lpf;
            unit: qsTr("Hz");
            precision: 2;
            value: 0;
            from: 0;
            width: parent.width;
            onValueChanged: {
                controller.set_imu_lpf(lpfcb.checked? value : 0);
            }
        }
    }
    CheckBoxWithContent {
        id: rot;
        text: qsTr("Rotation");
        //inner.visible: true;
        onCheckedChanged: update_rotation();
        function update_rotation() {
            controller.set_imu_rotation(rot.checked? p.value : 0, rot.checked? r.value : 0, rot.checked? y.value : 0);
        }

        Flow {
            width: parent.width;
            spacing: 5 * dpiScale;
            Label {
                position: Label.Left;
                text: qsTr("Pitch");
                width: undefined;
                inner.width: 50 * dpiScale;
                spacing: 5 * dpiScale;
                NumberField { id: p; unit: "°"; precision: 1; from: -360; to: 360; width: 50 * dpiScale; onValueChanged: rot.update_rotation(); tooltip: qsTr("Pitch is camera angle up/down when using FPV blackbox data"); }
            }
            Label {
                position: Label.Left;
                text: qsTr("Roll");
                width: undefined;
                inner.width: 50 * dpiScale;
                spacing: 5 * dpiScale;
                NumberField { id: r; unit: "°"; precision: 1; from: -360; to: 360; width: 50 * dpiScale; onValueChanged: rot.update_rotation(); }
            }
            Label {
                position: Label.Left;
                text: qsTr("Yaw");
                width: undefined;
                inner.width: 50 * dpiScale;
                spacing: 5 * dpiScale;
                NumberField { id: y; unit: "°"; precision: 1; from: -360; to: 360; width: 50 * dpiScale; onValueChanged: rot.update_rotation(); }
            }
        }
        /*BasicText {
            leftPadding: 0;
            width: parent.width;
            wrapMode: Text.WordWrap;
            font.pixelSize: 11 * dpiScale;
            text: qsTr("Pitch is camera angle up/down when using FPV blackbox data");
        }*/
    }
    Label {
        position: Label.Left;
        text: qsTr("IMU orientation");

        TextField {
            id: orientation;
            width: parent.width;
            text: "XYZ";
            validator: RegularExpressionValidator { regularExpression: /[XYZxyz]{3}/; }
            tooltip: qsTr("Uppercase is positive, lowercase is negative. eg. zYX");
            onTextChanged: if (acceptableInput) controller.set_imu_orientation(text);
        }
    }
    Label {
        position: Label.Left;
        text: qsTr("Integration method");

        ComboBox {
            id: integrator;
            property bool hasQuaternions: false;
            model: hasQuaternions? [QT_TRANSLATE_NOOP("Popup", "None"), "Madgwick", "Complementary", "Mahony", "Gyroflow"] :  ["Madgwick", "Complementary", "Mahony", "Gyroflow"];
            font.pixelSize: 12 * dpiScale;
            width: parent.width;
            tooltip: hasQuaternions && currentIndex === 0? qsTr("Use built-in quaternions instead of IMU data") : qsTr("IMU integration method for calculating motion data");
            onCurrentIndexChanged: {
                controller.set_integration_method(hasQuaternions? currentIndex : currentIndex + 1);
            }
            onHasQuaternionsChanged: {
                controller.set_integration_method(hasQuaternions? currentIndex : currentIndex + 1);
            }
        }
    }
    DropTarget {
        parent: root.innerItem;
        z: 999;
        anchors.rightMargin: -28 * dpiScale;
        anchors.topMargin: 35 * dpiScale;
        anchors.bottomMargin: -35 * dpiScale;
        extensions: fileDialog.extensions;
        onLoadFile: (url) => root.loadFile(url)
    }
}
