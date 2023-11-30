// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Dialogs

import "../components/"

MenuItem {
    id: root;
    text: qsTr("Motion data");
    iconName: "chart";
    loader: controller.loading_gyro_in_progress;
    objectName: "motiondata";

    property alias hasQuaternions: integrator.hasQuaternions;
    property bool hasAccurateTimestamps: false;
    property alias hasRawGyro: integrator.hasRawGyro;
    property alias integrationMethod: integrator.currentIndex;
    property alias orientationIndicator: orientationIndicator;
    property string filename: "";
    property url lastSelectedFile: "";

    FileDialog {
        id: fileDialog;
        property var extensions: [ "csv", "txt", "bbl", "bfl", "mp4", "mov", "mxf", "insv", "gcsv", "360", "log", "bin", "braw", "r3d" ];

        title: qsTr("Choose a motion data file")
        nameFilters: Qt.platform.os == "android"? undefined : [qsTr("Motion data files") + " (*." + extensions.concat(extensions.map(x => x.toUpperCase())).join(" *.") + ")"];
        type: "video";
        onAccepted: loadFile(selectedFile);
    }
    function loadFile(url: url) {
        if (!window.videoArea.vid.loaded) {
            messageBox(Modal.Error, qsTr("Video file is not loaded."), [ { text: qsTr("Ok"), accent: true } ]);
            return;
        }
        lastSelectedFile = url;
        controller.load_telemetry(url, false, window.videoArea.vid, currentLog.visible && currentLog.currentIndex > 0? currentLog.currentIndex - 1 : -1);
    }

    function loadGyroflow(obj) {
        const gyro = obj.gyro_source || { };
        if (gyro && Object.keys(gyro).length > 0) {
            if (gyro.rotation && gyro.rotation.length == 3) {
                p.value = gyro.rotation[0];
                r.value = gyro.rotation[1];
                y.value = gyro.rotation[2];
                rot.checked = Math.abs(p.value) > 0 || Math.abs(r.value) > 0 || Math.abs(y.value) > 0;
            }
            if (gyro.acc_rotation && gyro.acc_rotation.length == 3) {
                ap.value = gyro.acc_rotation[0];
                ar.value = gyro.acc_rotation[1];
                ay.value = gyro.acc_rotation[2];
                arot.checked = Math.abs(ap.value) > 0 || Math.abs(ar.value) > 0 || Math.abs(ay.value) > 0;
                arot_action.checked = arot.checked;
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
            if (typeof gyro.sample_index === "number") {
                currentLog.currentIndex = gyro.sample_index + 1;
            }
        }
    }
    function setGyroLpf(v: real) {
        lpf.value = v;
        lpfcb.checked = +v > 0;
    }

    function msToTime(ms: real): string {
        if (ms >= 60*60*1000) {
            return new Date(ms).toISOString().substring(11, 11+8);
        } else {
            return new Date(ms).toISOString().substring(11+3, 11+8);
        }
    }
    Connections {
        target: controller;
        function onTelemetry_loaded(is_main_video: bool, filename: string, camera: string, additional_data: var) {
            root.filename = filename || "";
            info.updateEntry("File name", filename || "---");
            info.updateEntry("Detected format", camera || "---");
            orientation.text = additional_data.imu_orientation;

            // Twice to trigger change signal
            integrator.hasRawGyro = additional_data.contains_raw_gyro;
            integrator.hasQuaternions = !additional_data.contains_quats;
            integrator.hasQuaternions = additional_data.contains_quats;
            root.hasAccurateTimestamps = additional_data.has_accurate_timestamps || false;
            if (additional_data.contains_quats && !is_main_video) {
                integrator.currentIndex = 2;
                integrateTimer.start();
            }
            if (!additional_data.contains_quats) {
                integrator.currentIndex = 1; // Default to VQF
            }

            controller.set_imu_lpf(lpfcb.checked? lpf.value : 0);
            controller.set_imu_rotation(rot.checked? p.value : 0, rot.checked? r.value : 0, rot.checked? y.value : 0);
            controller.set_acc_rotation(arot.checked? ap.value : 0, arot.checked? ar.value : 0, arot.checked? ay.value : 0);
            Qt.callLater(controller.recompute_gyro);

            Qt.callLater(window.videoArea.timeline.updateDurations);

            currentLog.preventChange = true;
            if (additional_data.usable_logs && additional_data.usable_logs.length > 0) {
                let model = ["All logs combined"];
                for (const log of additional_data.usable_logs) {
                    const [logIndex, startTimestamp, duration] = log.split(";");
                    model.push("#" + (+logIndex + 1) + " | " + msToTime(+startTimestamp) + " - " + msToTime(+startTimestamp + (+duration)) + " (" + msToTime(+duration) + ")");
                }
                if (currentLog.model != model)
                    currentLog.model = model;
            } else {
                currentLog.model = [];
            }
            currentLog.preventChange = false;
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
        function onChart_data_changed() {
            Qt.callLater(orientationIndicator.requestPaint);
        }
    }

    Button {
        text: qsTr("Open file");
        iconName: "file-empty"
        anchors.horizontalCenter: parent.horizontalCenter;
        onClicked: fileDialog.open2();
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
    Label {
        position: Label.LeftPosition;
        text: qsTr("Select log");
        visible: currentLog.count > 1;

        ComboBox {
            id: currentLog;
            property bool preventChange: false;
            model: [QT_TRANSLATE_NOOP("Popup", "All logs combined")];
            font.pixelSize: 12 * dpiScale;
            width: parent.width;
            onCurrentIndexChanged: {
                if (!preventChange && count > 1) {
                    root.loadFile(root.lastSelectedFile);
                }
            }
        }
    }
    CheckBoxWithContent {
        id: lpfcb;
        text: qsTr("Low pass filter");
        onCheckedChanged: {
            controller.set_imu_lpf(checked? lpf.value : 0);
            Qt.callLater(controller.recompute_gyro);
        }

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
                Qt.callLater(controller.recompute_gyro);
            }
        }
    }
    Item {
        width: parent.width;
        height: rot.height;
        CheckBoxWithContent {
            id: rot;
            text: qsTr("Rotation");
            onCheckedChanged: update_rotation();
            function update_rotation() {
                controller.set_imu_rotation(rot.checked? p.value : 0, rot.checked? r.value : 0, rot.checked? y.value : 0);
                Qt.callLater(controller.recompute_gyro);
            }
            ContextMenuMouseArea {
                parent: rot.cb;
                cursorShape: Qt.PointingHandCursor;
                onContextMenu: (isHold, x, y) => { contextMenu.popup(rot, x, y); }
            }

            Flow {
                width: parent.width;
                spacing: 5 * dpiScale;
                Label {
                    position: Label.LeftPosition;
                    text: qsTr("Pitch");
                    width: undefined;
                    inner.width: 50 * dpiScale;
                    spacing: 5 * dpiScale;
                    NumberField { id: p; unit: "°"; precision: 1; from: -360; to: 360; width: 50 * dpiScale; onValueChanged: rot.update_rotation(); tooltip: qsTr("Pitch is camera angle up/down when using FPV blackbox data"); }
                }
                Label {
                    position: Label.LeftPosition;
                    text: qsTr("Roll");
                    width: undefined;
                    inner.width: 50 * dpiScale;
                    spacing: 5 * dpiScale;
                    NumberField { id: r; unit: "°"; precision: 1; from: -360; to: 360; width: 50 * dpiScale; onValueChanged: rot.update_rotation(); }
                }
                Label {
                    position: Label.LeftPosition;
                    text: qsTr("Yaw");
                    width: undefined;
                    inner.width: 50 * dpiScale;
                    spacing: 5 * dpiScale;
                    NumberField { id: y; unit: "°"; precision: 1; from: -360; to: 360; width: 50 * dpiScale; onValueChanged: rot.update_rotation(); }
                }
            }
        }
        Menu {
            id: contextMenu;
            font.pixelSize: 11.5 * dpiScale;
            Action {
                id: arot_action;
                iconName: "axes";
                text: qsTr("Separate accelerometer rotation");
                checkable: true;
            }
        }
    }
    CheckBoxWithContent {
        id: arot;
        visible: arot_action.checked;
        text: qsTr("Accelerometer rotation");
        onCheckedChanged: update_rotation();
        function update_rotation() {
            controller.set_acc_rotation(arot.checked? ap.value : 0, arot.checked? ar.value : 0, arot.checked? ay.value : 0);
            Qt.callLater(controller.recompute_gyro);
        }

        Flow {
            width: parent.width;
            spacing: 5 * dpiScale;
            Label {
                position: Label.LeftPosition;
                text: qsTr("Pitch");
                width: undefined;
                inner.width: 50 * dpiScale;
                spacing: 5 * dpiScale;
                NumberField { id: ap; unit: "°"; precision: 1; from: -360; to: 360; width: 50 * dpiScale; onValueChanged: arot.update_rotation(); }
            }
            Label {
                position: Label.LeftPosition;
                text: qsTr("Roll");
                width: undefined;
                inner.width: 50 * dpiScale;
                spacing: 5 * dpiScale;
                NumberField { id: ar; unit: "°"; precision: 1; from: -360; to: 360; width: 50 * dpiScale; onValueChanged: arot.update_rotation(); }
            }
            Label {
                position: Label.LeftPosition;
                text: qsTr("Yaw");
                width: undefined;
                inner.width: 50 * dpiScale;
                spacing: 5 * dpiScale;
                NumberField { id: ay; unit: "°"; precision: 1; from: -360; to: 360; width: 50 * dpiScale; onValueChanged: arot.update_rotation(); }
            }
        }
    }
    CheckBoxWithContent {
        id: gyrobias;
        text: qsTr("Gyro bias");
        onCheckedChanged: update_bias();
        function update_bias() {
            controller.set_imu_bias(gyrobias.checked? bx.value : 0, gyrobias.checked? by.value : 0, gyrobias.checked? bz.value : 0);
            Qt.callLater(controller.recompute_gyro);
        }

        Flow {
            width: parent.width;
            spacing: 5 * dpiScale;
            Label {
                position: Label.LeftPosition;
                text: qsTr("X");
                width: undefined;
                inner.width: 65 * dpiScale;
                spacing: 5 * dpiScale;
                NumberField { id: bx; unit: "°/s"; precision: 2; width: 65 * dpiScale; onValueChanged: gyrobias.update_bias(); }
            }
            Label {
                position: Label.LeftPosition;
                text: qsTr("Y");
                width: undefined;
                inner.width: 65 * dpiScale;
                spacing: 5 * dpiScale;
                NumberField { id: by; unit: "°/s"; precision: 2; width: 65 * dpiScale; onValueChanged: gyrobias.update_bias(); }
            }
            Label {
                position: Label.LeftPosition;
                text: qsTr("Z");
                width: undefined;
                inner.width: 65 * dpiScale;
                spacing: 5 * dpiScale;
                NumberField { id: bz; unit: "°/s"; precision: 2; width: 65 * dpiScale; onValueChanged: gyrobias.update_bias(); }
            }
        }
    }
    Label {
        position: Label.LeftPosition;
        text: qsTr("IMU orientation");

        TextField {
            id: orientation;
            width: parent.width;
            text: "XYZ";
            validator: RegularExpressionValidator { regularExpression: /[XYZxyz]{3}/; }
            tooltip: qsTr("Uppercase is positive, lowercase is negative. eg. zYX");
            onTextChanged: if (acceptableInput) { controller.set_imu_orientation(text); Qt.callLater(controller.recompute_gyro); }
        }
    }
    Label {
        position: Label.LeftPosition;
        text: qsTr("Integration method");

        ComboBox {
            id: integrator;
            property bool hasQuaternions: false;
            property bool hasRawGyro: false;
            model: hasQuaternions? [QT_TRANSLATE_NOOP("Popup", "None"), "Complementary", "VQF", "Simple gyro", "Simple gyro + accel", "Mahony", "Madgwick" ] : ["Complementary", "VQF", "Simple gyro", "Simple gyro + accel", "Mahony", "Madgwick"];
            font.pixelSize: 12 * dpiScale;
            width: parent.width;
            tooltip: hasQuaternions && currentIndex === 0? qsTr("Use built-in quaternions instead of IMU data") : qsTr("IMU integration method for calculating motion data");
            function setMethod() {
                controller.set_integration_method(hasQuaternions? currentIndex : currentIndex + 1);
            }
            onCurrentIndexChanged: integrateTimer.start();
            onHasQuaternionsChanged: integrateTimer.start();
            Timer {
                id: integrateTimer;
                interval: 300;
                onTriggered: Qt.callLater(integrator.setMethod);
            }
        }
    }

    CheckBoxWithContent {
        id: orientationCheckbox;
        text: qsTr("Orientation indicator");
        onCheckedChanged: Qt.callLater(orientationIndicator.requestPaint);

        Canvas {
            id: orientationIndicator
            width: parent.width
            height: 100
            property var currentTimestamp: 0
            property var initialDraw: false
            onPaint: {
                if (orientationCheckbox.checked || !initialDraw) {
                    initialDraw = true
                    let ctx = getContext("2d");
                    ctx.reset();
                    const veclen = 30;
                    const xv = Qt.vector3d(0,veclen,0)
                    const yv = Qt.vector3d(-veclen,0,0)
                    const zv = Qt.vector3d(0,0,veclen)
                    const vecs = [xv, yv, zv]
                    const colors = style === "light" ? ['#cc0000', '#00cc00', '#0000cc'] : ['#ff0000', '#00ff00', '#4444ff'];
                    // inspired by blender camera
                    const cam_width = 30;
                    const cam_height = 15;
                    const cam_length = 30;
                    const cam_vertices = [[-cam_width,-cam_height,-cam_length],
                                          [cam_width, -cam_height,-cam_length],
                                          [cam_width, cam_height, -cam_length],
                                          [-cam_width, cam_height, -cam_length],
                                          [0,0,0]]
                    const cam_vert_vecs = cam_vertices.map(vert => Qt.vector3d(vert[0],vert[1],vert[2]))
                    const lines = [[0,1,2,3,0],
                                   [0,4,1],
                                   [2,4,3]]

                    const quats = controller.quats_at_timestamp(Math.round(currentTimestamp))
                    const transform = Qt.quaternion( quats[0], quats[1],  quats[2], quats[3]); // wxyz
                    const maincolor = style === "light" ? "rgba(0,0,0,0.9)" : "rgba(255,255,255,0.9)";
                    const transform_smooth = transform.times(Qt.quaternion( quats[4], quats[5],  quats[6], quats[7]).inverted());
                    const transforms = [transform, transform_smooth]

                    // center dots
                    for (let i = 0; i < 3; i++) {
                        ctx.beginPath();
                        ctx.arc(width/6*(i*2+1), height/2, 4, 0, 2 * Math.PI, false);
                        ctx.fillStyle = maincolor;
                        ctx.fill();
                        ctx.stroke();
                    }

                    for (let i = 0; i < 3; i++) {
                        ctx.beginPath();
                        ctx.moveTo(width/6, height/2);
                        const transformedvec = transform.times(vecs[i])
                        ctx.lineTo(width/6 + transformedvec.x, height/2 - transformedvec.y);
                        ctx.lineWidth = 3;
                        ctx.strokeStyle = colors[i];
                        ctx.globalAlpha = 0.5;
                        ctx.stroke();
                        ctx.globalAlpha = Math.max(0.1, Math.min(transformedvec.z/(veclen*2)+0.5,1));
                        ctx.beginPath();
                        ctx.arc(width/6 + transformedvec.x, height/2 - transformedvec.y, 4, 0, 2 * Math.PI, false);
                        ctx.fillStyle = colors[i];
                        ctx.fill();
                        ctx.stroke();
                    }

                    ctx.lineWidth = 1.5;
                    ctx.strokeStyle = maincolor;
                    ctx.globalAlpha = 0.8;
                    ctx.lineJoin = "bevel";
                    for (let view = 0; view < 2; view++) {
                        for (let linenum = 0; linenum < lines.length; linenum++) {
                            ctx.beginPath()
                            for (let pointnum=0; pointnum < lines[linenum].length; pointnum++) {
                                const transformedvec = transforms[view].times(cam_vert_vecs[lines[linenum][pointnum]]);
                                if (pointnum == 0) {
                                    ctx.moveTo(transformedvec.x + width/6*(view*2 + 3), -transformedvec.y + height/2);
                                }
                                else {
                                    ctx.lineTo(transformedvec.x + width/6*(view*2 + 3), -transformedvec.y + height/2);
                                }
                            }
                            ctx.stroke();
                        }
                    }
                }
            }
            function updateOrientation(timestamp) {
                currentTimestamp = timestamp;
                requestPaint();
            }
        }
    }

    LinkButton {
        text: qsTr("Statistics");
        anchors.horizontalCenter: parent.horizontalCenter;
        //anchors.verticalCenter: parent.verticalCenter;
        onClicked: {
            if (window.videoArea.statistics.item) window.videoArea.statistics.item.shown = !window.videoArea.statistics.item.shown;
            window.videoArea.statistics.active = true;
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
