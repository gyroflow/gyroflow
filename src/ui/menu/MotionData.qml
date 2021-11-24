import QtQuick 2.15
import QtQuick.Dialogs

import "../components/"

MenuItem {
    id: root;
    text: qsTr("Motion data");
    icon: "chart";

    FileDialog {
        id: fileDialog;
        property var extensions: ["csv", "txt", "bbl", "bfl", "mp4", "mov", "mxf", "360", "gyroflow"];

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
        function onTelemetry_loaded(is_main_video, filename, camera, imu_orientation, contains_gyro, contains_quats, frame_readout_time) {
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
        model: ({
            "File name": "---",
            "Detected format": "---"
        })
    }
    CheckBoxWithContent {
        text: qsTr("Low pass filter");
        onCheckedChanged: controller.update_lpf(checked? lpf.value : 0);

        NumberField {
            id: lpf;
            unit: "Hz";
            precision: 2;
            value: 0;
            from: 0;
            width: parent.width;
            onValueChanged: {
                controller.update_lpf(value);
            }
        }
    }
    CheckBoxWithContent {
        id: rot;
        text: qsTr("Rotation");
        //inner.visible: true;
        function update_rotation() {
            console.log('update_rotation', p.value, r.value, y.value);
            controller.update_imu_rotation(p.value, r.value, y.value);
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
            onTextChanged: if (acceptableInput) controller.update_imu_orientation(text);
        }
    }
    Label {
        position: Label.Left;
        text: qsTr("Integration method");

        ComboBox {
            id: integrator;
            property bool hasQuaternions: false;
            model: hasQuaternions? [qsTr("None"), "Madgwick", "Complementary", "Mahony", "Gyroflow"] :  ["Madgwick", "Complementary", "Mahony", "Gyroflow"];
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
