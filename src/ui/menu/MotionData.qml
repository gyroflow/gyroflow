// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Dialogs

import "../components/"

MenuItem {
    id: root;
    text: qsTr("Motion data");
    icon: "chart";
    loader: controller.loading_gyro_in_progress;
    objectName: "motiondata";

    property alias hasQuaternions: integrator.hasQuaternions;
    property alias integrationMethod: integrator.currentIndex;
    property string filename: "";

    property var pendingOffsets: ({});

    FileDialog {
        id: fileDialog;
        property var extensions: [
            "csv", "txt", "bbl", "bfl", "mp4", "mov", "mxf", "insv", "gcsv", "360",
            "CSV", "TXT", "BBL", "BFL", "MP4", "MOV", "MXF", "INSV", "GCSV", "log"
        ];

        title: qsTr("Choose a motion data file")
        nameFilters: Qt.platform.os == "android"? undefined : [qsTr("Motion data files") + " (*." + extensions.join(" *.") + ")"];
        onAccepted: loadFile(selectedFile);
    }
    function loadFile(url: url) {
        root.pendingOffsets = { };
        if (Qt.platform.os == "android") {
            url = Qt.resolvedUrl("file://" + controller.resolve_android_url(url.toString()));
        }
        controller.load_telemetry(url, false, window.videoArea.vid, window.videoArea.timeline.getChart());
    }

    function loadGyroflow(obj) {
        const gyro = obj.gyro_source || { };
        if (gyro && Object.keys(gyro).length > 0) {
            if (gyro.filepath && (gyro.filepath != obj.videofile) && controller.file_exists(gyro.filepath)) {
                loadFile(controller.path_to_url(gyro.filepath));
                root.pendingOffsets = obj.offsets; // because loading gyro data will clear offsets
            }
            if (gyro.rotation && gyro.rotation.length == 3) {
                p.value = gyro.rotation[0];
                r.value = gyro.rotation[1];
                y.value = gyro.rotation[2];
                rot.checked = Math.abs(p.value) > 0 || Math.abs(r.value) > 0 || Math.abs(y.value) > 0;
            }
            if (gyro.imu_orientation) orientation.text = gyro.imu_orientation;
            if (gyro.hasOwnProperty("integration_method")) {
                const index = +gyro.integration_method;
                integrator.currentIndex = integrator.hasQuaternions? index : index - 1;
            }
            if (+gyro.lpf > 0) {
                lpf.value = +gyro.lpf;
                lpfcb.checked = lpf.value > 0;
            }
        }
    }
    function setGyroLpf(v: real) {
        lpf.value = v;
        lpfcb.checked = +v > 0;
    }

    Connections {
        target: controller;
        function onTelemetry_loaded(is_main_video: bool, filename: string, camera: string, imu_orientation: string, contains_gyro: bool, contains_quats: bool, frame_readout_time: real, camera_id_json: string) {
            root.filename = filename || "";
            info.updateEntry("File name", filename || "---");
            info.updateEntry("Detected format", camera || "---");
            orientation.text = imu_orientation;

            // Twice to trigger change signal
            integrator.hasQuaternions = !contains_quats;
            integrator.hasQuaternions = contains_quats;
            if (integrator.hasQuaternions && !is_main_video) {
                Qt.callLater(() => integrator.currentIndex = 1);
            }

            const chart = window.videoArea.timeline.getChart();
            chart.setDurationMs(controller.get_scaled_duration_ms());
            window.videoArea.durationMs = controller.get_scaled_duration_ms();

            controller.set_imu_lpf(lpfcb.checked? lpf.value : 0);
            controller.set_imu_rotation(rot.checked? p.value : 0, rot.checked? r.value : 0, rot.checked? y.value : 0);

            Qt.callLater(() => controller.update_chart(window.videoArea.timeline.getChart()));
            if (root.pendingOffsets) {
                for (const ts in root.pendingOffsets) {
                    controller.set_offset(ts, root.pendingOffsets[ts]);
                }
                root.pendingOffsets = {};
            }
        }
        function onBias_estimated(biasX: real, biasY: real, biasZ: real) {
            gyrobias.checked = true;
            bx.value = biasX;
            by.value = biasY;
            bz.value = biasZ;
        }
        function onOrientation_guessed(value: string) {
             orientation.text = value;
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
            value: 50;
            from: 0;
            width: parent.width;
            tooltip: qsTr("Lower cutoff frequency means more filtering");
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
    CheckBoxWithContent {
        id: gyrobias;
        text: qsTr("Gyro bias");
        onCheckedChanged: update_bias();
        function update_bias() {
            Qt.callLater(controller.set_imu_bias, gyrobias.checked? bx.value : 0, gyrobias.checked? by.value : 0, gyrobias.checked? bz.value : 0);
        }

        Flow {
            width: parent.width;
            spacing: 5 * dpiScale;
            Label {
                position: Label.Left;
                text: qsTr("X");
                width: undefined;
                inner.width: 65 * dpiScale;
                spacing: 5 * dpiScale;
                NumberField { id: bx; unit: "°/s"; precision: 2; width: 65 * dpiScale; onValueChanged: gyrobias.update_bias(); }
            }
            Label {
                position: Label.Left;
                text: qsTr("Y");
                width: undefined;
                inner.width: 65 * dpiScale;
                spacing: 5 * dpiScale;
                NumberField { id: by; unit: "°/s"; precision: 2; width: 65 * dpiScale; onValueChanged: gyrobias.update_bias(); }
            }
            Label {
                position: Label.Left;
                text: qsTr("Z");
                width: undefined;
                inner.width: 65 * dpiScale;
                spacing: 5 * dpiScale;
                NumberField { id: bz; unit: "°/s"; precision: 2; width: 65 * dpiScale; onValueChanged: gyrobias.update_bias(); }
            }
        }
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
            model: hasQuaternions? [QT_TRANSLATE_NOOP("Popup", "None"), "Complementary", "Madgwick", "Mahony", "Gyroflow"] :  ["Complementary", "Madgwick", "Mahony", "Gyroflow"];
            font.pixelSize: 12 * dpiScale;
            width: parent.width;
            tooltip: hasQuaternions && currentIndex === 0? qsTr("Use built-in quaternions instead of IMU data") : qsTr("IMU integration method for calculating motion data");
            function setMethod() {
                controller.set_integration_method(hasQuaternions? currentIndex : currentIndex + 1);
            }
            onCurrentIndexChanged: Qt.callLater(integrator.setMethod);
            onHasQuaternionsChanged: Qt.callLater(integrator.setMethod);
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
        onLoadFile: (url) => root.loadFile(url)
    }
}
