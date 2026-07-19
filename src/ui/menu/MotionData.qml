// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>
// Copyright © 2026 dan0v <dev@dan0v.com>

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
    property bool allMetadata: allMetadataCb.visible && allMetadataCb.checked;
    property string filename: "";
    property string detectedFormat: "";
    property url lastSelectedFile: "";

    FileDialog {
        id: fileDialog;
        property var extensions: [ "csv", "txt", "bbl", "bfl", "mp4", "mov", "mxf", "insv", "gcsv", "360", "log", "bin", "braw", "r3d", "nev", "gpmf", "crm" ];

        title: qsTr("Choose a motion data file")
        nameFilters: Qt.platform.os == "android"? undefined : [qsTr("Motion data files") + " (*." + extensions.concat(extensions.map(x => x.toUpperCase())).join(" *.") + ")"];
        type: "video";
        onAccepted: loadFile(selectedFile);
    }
    function loadFile(url: url): void {
        if (!window.videoArea.vid.loaded) {
            messageBox(Modal.Error, qsTr("Video file is not loaded."), [ { text: qsTr("Ok"), accent: true } ]);
            return;
        }
        lastSelectedFile = url;
        controller.load_telemetry(url, root.allMetadata, window.videoArea.vid, currentLog.visible && currentLog.currentIndex > 0? currentLog.currentIndex - 1 : -1, 0);
    }

    function loadGyroflow(obj: var): void {
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
            if (typeof gyro.glitch_strength === "number" && +gyro.glitch_strength > 0) {
                glitchStrength.value = +gyro.glitch_strength;
            }
            if (gyro.hasOwnProperty("glitch_filter")) {
                glitchcb.checked = !!gyro.glitch_filter;
            }
            if (typeof gyro.sample_index === "number") {
                currentLog.currentIndex = gyro.sample_index + 1;
            }
        }
        const stab = obj.stabilization || { };
        if (stab && Object.keys(stab).length > 0) {
            focb.checked = +stab.frame_offset > 0;
            fo.value = +stab.frame_offset;
        }
    }
    function setGyroLpf(v: real): void {
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
        function onTelemetry_loaded(is_main_video: bool, filename: string, camera: string, additional_data: var): void {
            root.filename = filename || "";
            root.detectedFormat = camera || "";
            info.updateEntry("File name", filename || "---");
            info.updateEntry("Detected format", camera || "---");
            orientation.text = additional_data.imu_orientation;

            // Twice to trigger change signal
            integrator.hasRawGyro = additional_data.contains_raw_gyro;
            integrator.hasQuaternions = !additional_data.contains_quats;
            integrator.hasQuaternions = additional_data.contains_quats;
            root.hasAccurateTimestamps = additional_data.has_accurate_timestamps || false;
            if (additional_data.contains_quats && !is_main_video) {
                if (integrator.hasRawGyro) {
                    integrator.currentIndex = 2;
                } else {
                    integrator.currentIndex = 0;
                }
                integrateTimer.start();
            }
            if (!additional_data.contains_quats) {
                integrator.currentIndex = 1; // Default to VQF
                // Default to Complementary if video is shorter than 10s
                if (controller.get_scaled_duration_ms() < 10000) {
                    integrator.currentIndex = 0;
                }
            }

            controller.set_imu_lpf(lpfcb.checked? lpf.value : 0);
            controller.set_imu_median_filter(mfcb.checked? mf.value : 0);
            controller.set_glitch_filter(glitchcb.checked, glitchStrength.value);
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
        function onBias_estimated(biasX: real, biasY: real, biasZ: real): void {
            gyrobias.checked = true;
            bx.value = biasX;
            by.value = biasY;
            bz.value = biasZ;
        }
        function onOrientation_guessed(value: string): void {
             orientation.text = value;
        }
        function onChart_data_changed(): void {
            Qt.callLater(orientationIndicator.requestPaint);
        }
    }

    Button {
        text: qsTr("Open file");
        iconName: "file-empty"
        anchors.horizontalCenter: parent.horizontalCenter;
        onClicked: fileDialog.open2();
    }
    InfoMessageSmall {
        show: Qt.platform.os == "android" && !root.detectedFormat && root.lastSelectedFile.toString();
        type: InfoMessage.Info;
        text: qsTr("In order to detect multiple motion data files, click here and grant access to the directory with files.");
        OutputPathField { id: opf; visible: false; }
        MouseArea {
            anchors.fill: parent;
            cursorShape: Qt.PointingHandCursor;
            onClicked: {
                opf.selectFolder("", function(_) {
                    root.loadFile(root.lastSelectedFile);
                });
            }
        }
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
    CheckBox {
        id: allMetadataCb;
        text: qsTr("Load all metadata");
        visible: root.lastSelectedFile.toString() && root.lastSelectedFile != window.videoArea.loadedFileUrl;
        checked: false;
        onCheckedChanged: allMetadataDebounce.start();
        Timer {
            id: allMetadataDebounce;
            interval: 1;
            running: false;
            repeat: false;
            onTriggered: root.loadFile(root.lastSelectedFile);
        }
    }
    CheckBoxWithContent {
        id: focb;
        text: qsTr("Frame offset");
        visible: allMetadataCb.visible;
        onCheckedChanged: {
            controller.frame_offset = focb.checked? fo.value : 0;
        }
        NumberField {
            id: fo;
            unit: qsTr("frames");
            precision: 0;
            value: 0;
            from: -100000;
            to: 100000;
            width: parent.width;
            tooltip: qsTr("Add or subtract frames from the video to align with motion data");
            onValueChanged: {
                controller.frame_offset = focb.checked? fo.value : 0;
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
    CheckBoxWithContent {
        id: mfcb;
        text: qsTr("Median filter");
        onCheckedChanged: {
            controller.set_imu_median_filter(checked? mf.value : 0);
            Qt.callLater(controller.recompute_gyro);
        }

        NumberField {
            id: mf;
            unit: qsTr("samples");
            precision: 0;
            value: 5;
            from: 0;
            width: parent.width;
            onValueChanged: {
                controller.set_imu_median_filter(mfcb.checked? value : 0);
                Qt.callLater(controller.recompute_gyro);
            }
        }
    }
    CheckBoxWithContent {
        id: glitchcb;
        text: qsTr("Glitch filtering");
        tooltip: qsTr("Detect and repair short bursts of corrupt gyro data");
        onCheckedChanged: {
            controller.set_glitch_filter(checked, glitchStrength.value);
            Qt.callLater(controller.recompute_gyro);
        }
        Label {
            text: qsTr("Strength");
            width: parent.width;
            tooltip: qsTr("Higher values detect glitches more aggressively (catching weaker and longer bursts with more passes), but may affect real fast motion. Lower values only repair the obvious, large glitches. 50% is the default.");
            SliderWithField {
                id: glitchStrength;
                defaultValue: 50;
                to: 100;
                value: 50;
                unit: "%";
                precision: 0;
                width: parent.width;
                onValueChanged: {
                    controller.set_glitch_filter(glitchcb.checked, value);
                    Qt.callLater(controller.recompute_gyro);
                }
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
            function update_rotation(): void {
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
        function update_rotation(): void {
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
        function update_bias(): void {
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
            function setMethod(): void {
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
            property real currentTimestamp: 0
            property bool initialDraw: false
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
            function updateOrientation(timestamp: real): void {
                currentTimestamp = timestamp;
                requestPaint();
                meshCorrection.requestPaint();
                focalPlaneDistortion.requestPaint();
            }
        }
        Canvas {
            id: meshCorrection
            width: parent.width
            height: width * (orgH / orgW);
            visible: false;

            property real orgW: window.videoArea.outWidth  || window.videoArea.vid.videoWidth;
            property real orgH: window.videoArea.outHeight || window.videoArea.vid.videoHeight;
            property bool initialDraw: false
            onPaint: {
                if (orientationCheckbox.checked || !initialDraw) {
                    initialDraw = true
                    let ctx = getContext("2d");
                    ctx.reset();
                    const maincolor = style === "light" ? "rgba(0,0,0,0.9)" : "rgba(255,255,255,0.9)";
                    const margin = 15 * dpiScale;

                    const mesh = controller.mesh_at_frame(window.videoArea.vid.currentFrame);
                    if (!mesh.length || mesh[0] < 10) { meshCorrection.visible = false; return; }
                    const divisions = [mesh[1], mesh[2]];
                    const mesh_size = [mesh[3], mesh[4]];

                    meshCorrection.visible = divisions[0] > 0;

                    for (let i = 0; i < (divisions[0]*divisions[1]*2); i += 2) {
                        const x = margin + (width  - 2*margin) * mesh[9 + i + 0] / mesh_size[0];
                        const y = margin + (height - 2*margin) * mesh[9 + i + 1] / mesh_size[1];
                        ctx.beginPath();
                        ctx.arc(x, y, 2 * dpiScale, 0, 2 * Math.PI, false);
                        ctx.fillStyle = maincolor;
                        ctx.fill();
                    }

                    ctx.lineWidth = 1 * dpiScale;
                    ctx.strokeStyle = maincolor;
                    ctx.beginPath();
                    for (let i = 0; i < (divisions[0]*divisions[1]*2); i += 2) {
                        const x = margin + (width  - 2*margin) * (mesh[9 + i + 0] / mesh_size[0]);
                        const y = margin + (height - 2*margin) * (mesh[9 + i + 1] / mesh_size[1]);
                        ctx.moveTo(x, y);
                        if (((i + 2) / 2) % divisions[1] != 0) {
                            const xx = margin + (width  - 2*margin) * mesh[9 + i + 2 + 0] / mesh_size[0];
                            const yy = margin + (height - 2*margin) * mesh[9 + i + 2 + 1] / mesh_size[1];
                            ctx.lineTo(xx, yy);
                        }
                        if (i + divisions[1]*2 < divisions[0]*divisions[1]*2) {
                            const xxx = margin + (width  - 2*margin) * (mesh[9 + i + divisions[0]*2 + 0] / mesh_size[0]);
                            const yyy = margin + (height - 2*margin) * (mesh[9 + i + divisions[0]*2 + 1] / mesh_size[1]);
                            ctx.moveTo(x, y);
                            ctx.lineTo(xxx, yyy);
                        }
                    }
                    ctx.stroke();
                }
            }
        }
        Canvas {
            id: focalPlaneDistortion;
            width: parent.width
            height: width * (orgH / orgW);
            visible: false;

            property real orgW: window.videoArea.outWidth  || window.videoArea.vid.videoWidth;
            property real orgH: window.videoArea.outHeight || window.videoArea.vid.videoHeight;
            property bool initialDraw: false
            onPaint: {
                if (orientationCheckbox.checked || !initialDraw) {
                    initialDraw = true
                    let ctx = getContext("2d");
                    ctx.reset();
                    const maincolor = style === "light" ? "rgba(0,0,0,0.9)" : "rgba(255,255,255,0.9)";
                    const margin = 15 * dpiScale;

                    const mesh = controller.mesh_at_frame(window.videoArea.vid.currentFrame);
                    if (!mesh.length || mesh[0] == 0 || mesh[mesh[0]] == 0) { focalPlaneDistortion.visible = false; return; }
                    const mesh_size = [mesh[3], mesh[4]];

                    focalPlaneDistortion.visible = true;

                    ctx.lineWidth = 1 * dpiScale;
                    ctx.strokeStyle = maincolor;
                    const o = mesh[0];
                    const stblz_grid = mesh_size[1] / 8;

                    let points = [];
                    for (let i = 0; i < 8; ++i) {
                        // corners of the rectangle
                        points.push([0, i * stblz_grid]);
                        points.push([mesh_size[0], i * stblz_grid]);
                        points.push([mesh_size[0], (i + 1) * stblz_grid]);
                        points.push([0, (i + 1) * stblz_grid]);

                        points.push([0, i * stblz_grid]);
                    }

                    for (let i = 0; i < points.length; ++i) {
                        const idx = Math.min(7, Math.max(0, Math.floor(points[i][1] / stblz_grid)));
                        const delta = points[i][1] - stblz_grid * idx;
                        points[i][0] += mesh[o + 4 + idx * 2 + 0] * delta;
                        points[i][1] += mesh[o + 4 + idx * 2 + 1] * delta;
                        for (let j = 0; j < idx; j++) {
                            points[i][0] += mesh[o + 4 + j * 2 + 0] * stblz_grid;
                            points[i][1] += mesh[o + 4 + j * 2 + 1] * stblz_grid;
                        }
                    }

                    ctx.beginPath();
                    for (let i = 0; i < points.length; ++i) {
                        const x = margin + (width  - 2*margin) * points[i][0] / mesh_size[0];
                        const y = margin + (height - 2*margin) * points[i][1] / mesh_size[1];
                        if (i == 0) ctx.moveTo(x, y);
                        else ctx.lineTo(x, y);
                    }
                    ctx.stroke();
                }
            }
        }
    }

    Row {
        anchors.horizontalCenter: parent.horizontalCenter;
        LinkButton {
            text: qsTr("Statistics");
            onClicked: {
                if (window.videoArea.statistics.item) window.videoArea.statistics.item.shown = !window.videoArea.statistics.item.shown;
                window.videoArea.statistics.active = true;
            }
        }
        LinkButton {
            id: exportGyroBtn;
            text: qsTr("Export");
            onClicked: {
                menuLoader.toggle(exportGyroBtn, 0, height);
            }
            Component {
                id: menu;
                Menu {
                    id: menuInner;
                    FileDialog {
                        id: exportFileDialog;
                        fileMode: FileDialog.SaveFile;
                        title: qsTr("Select file destination");
                        type: "gyro-csv";
                        property var exportData: ({});
                        onAccepted: {
                            if (exportData === "full") {
                                controller.export_full_metadata(selectedFile, root.lastSelectedFile.toString()? root.lastSelectedFile : window.videoArea.loadedFileUrl);
                            } else if (exportData == "parsed") {
                                controller.export_parsed_metadata(selectedFile);
                            } else {
                                controller.export_gyro_data(selectedFile, exportData);
                            }
                        }
                    }
                    Action {
                        text: qsTr("Export camera data (CSV/JSON/USD/AE)");
                        onTriggered: {
                            const el = Qt.createComponent("../SettingsSelector.qml").createObject(window, {
                                desc: [
                                    {
                                        "Original|original": {
                                            "Gyroscope":       ["gyroscope"],
                                            "Accelerometer":   ["accelerometer"],
                                            "Quaternion":      ["quaternion"],
                                            "Euler angles":    ["euler_angles"],
                                            "Focus distances": ["focus_distances"],
                                        },
                                    },
                                    {
                                        "Stabilized|stabilized": {
                                            "Quaternion":    ["quaternion"],
                                            "Euler angles":  ["euler_angles"],
                                        },
                                    },
                                    {
                                        "Zooming|zooming": {
                                            "Minimal FOV scale":  ["minimal_fovs"],
                                            "Smoothed FOV scale": ["fovs"],
                                            "Focal length (if available)": ["focal_length"],
                                        },
                                    }
                                ],
                                type: "gyro_csv"
                            });
                            let savedState = settings.value("CSVExportSelection", "");
                            if (savedState) {
                                try {
                                    el.loadSelection(JSON.parse(savedState));
                                } catch(e) { }
                            }
                            el.opened = true;
                            el.onApply.connect((obj) => {
                                settings.setValue("CSVExportSelection", JSON.stringify(obj));

                                if (Qt.platform.os == "ios") {
                                    const exportToFolder = ext => {
                                        const folder = filesystem.get_folder(root.lastSelectedFile.toString()? root.lastSelectedFile : window.videoArea.loadedFileUrl);
                                        const opf = Qt.createComponent("../components/OutputPathField.qml").createObject(window, { visible: false });
                                        opf.selectFolder(folder, function(folder_url) {
                                            const filename = root.filename.replace(/\.[^/.]+$/, "." + ext);
                                            controller.export_gyro_data(filesystem.get_file_url(folder_url, filename, true), obj);
                                            opf.destroy();
                                        });
                                    };
                                    messageBox(Modal.Question, qsTr("Which format do you want to use?"), [
                                        { text: "CSV",                         clicked: function() { exportToFolder("csv"); } },
                                        { text: "JSON", accent: true,          clicked: function() { exportToFolder("json"); } },
                                        { text: "Universal Scene Description", clicked: function() { exportToFolder("usd"); } },
                                        { text: "After Effects Script",        clicked: function() { exportToFolder("jsx"); } },
                                        { text: qsTr("Cancel") },
                                    ]);
                                    return;
                                }

                                exportFileDialog.nameFilters = ["CSV (*.csv)", "JSON (*.json)", "Universal Scene Description (*.usd)", "After Effects Script (*.jsx)"];
                                exportFileDialog.exportData = obj;
                                exportFileDialog.open2();
                            });
                        }
                    }
                    Action {
                        text: qsTr("Export full metadata");
                        onTriggered: {
                            if (Qt.platform.os == "ios") {
                                const folder = filesystem.get_folder(root.lastSelectedFile.toString()? root.lastSelectedFile : window.videoArea.loadedFileUrl);
                                const opf = Qt.createComponent("../components/OutputPathField.qml").createObject(window, { visible: false });
                                opf.selectFolder(folder, function(folder_url) {
                                    const filename = root.filename.replace(/\.[^/.]+$/, ".json");
                                    controller.export_full_metadata(filesystem.get_file_url(folder_url, filename, true), root.lastSelectedFile.toString()? root.lastSelectedFile : window.videoArea.loadedFileUrl);
                                    opf.destroy();
                                });
                                return;
                            }

                            exportFileDialog.nameFilters = ["JSON (*.json)"];
                            exportFileDialog.exportData = "full";
                            exportFileDialog.open2();
                        }
                    }
                    Action {
                        text: qsTr("Export parsed metadata");
                        onTriggered: {
                            if (Qt.platform.os == "ios") {
                                const folder = filesystem.get_folder(root.lastSelectedFile.toString()? root.lastSelectedFile : window.videoArea.loadedFileUrl);
                                const opf = Qt.createComponent("../components/OutputPathField.qml").createObject(window, { visible: false });
                                opf.selectFolder(folder, function(folder_url) {
                                    const filename = root.filename.replace(/\.[^/.]+$/, ".json");
                                    controller.export_parsed_metadata(filesystem.get_file_url(folder_url, filename, true));
                                    opf.destroy();
                                });
                                return;
                            }
                            exportFileDialog.nameFilters = ["JSON (*.json)"];
                            exportFileDialog.exportData = "parsed";
                            exportFileDialog.open2();
                        }
                    }
                    Action {
                        text: qsTr("Export project file (including processed gyro data)");
                        onTriggered: window.saveProject("WithProcessedData");
                    }
                }
            }
            ContextMenuLoader {
                id: menuLoader;
                sourceComponent: menu
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
        onLoadFile: (url) => root.loadFile(url)
    }

    Column {
        id: repairSection;
        width: parent.width;
        spacing: 5 * dpiScale;
        visible: controller.gyro_loaded;
        property real gyroRepairProgress: 0;
        // Theme-aware repair-region palette (orange accent, dimmed when disabled).
        readonly property color regionBorder:      style === "light" ? "#ff6600" : "#ff6600";
        readonly property color regionBorderSelected: "#ffaa00";
        readonly property color regionFillSelected: style === "light" ? "#60ff6600" : "#60ff6600";
        readonly property color regionFillEnabled:  style === "light" ? "#30ff6600" : "#30ff6600";
        readonly property color regionFillDisabled: style === "light" ? "#18000000" : "#18000000";
        readonly property color rowBgSelected:     style === "light" ? "#20000000" : "#15ffffff";
        readonly property color rowBgUnselected:   style === "light" ? "#08000000" : "#08ffffff";

        BasicText {
            text: qsTr("Gyro repair");
            font.bold: true;
            font.pixelSize: 13 * dpiScale;
        }
        BasicText {
            text: qsTr("Select a time range with corrupted gyro data and replace it with optical flow analysis.");
            width: parent.width;
            font.pixelSize: 11 * dpiScale;
            wrapMode: Text.WordWrap;
        }
        Label {
            position: Label.LeftPosition;
            text: qsTr("Region start");
            inner.width: 110 * dpiScale;
            Row {
                spacing: 5 * dpiScale;
                NumberField {
                    id: repairStart;
                    unit: "ms";
                    precision: 1;
                    from: 0;
                    to: 999999;
                    width: 75 * dpiScale;
                    value: 0;
                }
                Button {
                    text: "[";
                    font.bold: true;
                    width: 30 * dpiScale;
                    leftPadding: 3 * dpiScale;
                    rightPadding: 3 * dpiScale;
                    tooltip: qsTr("Set to current cursor position");
                    onClicked: repairStart.value = Math.round(window.videoArea.vid.timestamp * 10) / 10;
                }
            }
        }
        Label {
            position: Label.LeftPosition;
            text: qsTr("Region end");
            inner.width: 110 * dpiScale;
            Row {
                spacing: 5 * dpiScale;
                NumberField {
                    id: repairEnd;
                    unit: "ms";
                    precision: 1;
                    from: 0;
                    to: 999999;
                    width: 75 * dpiScale;
                    value: 0;
                }
                Button {
                    text: "]";
                    font.bold: true;
                    width: 30 * dpiScale;
                    leftPadding: 3 * dpiScale;
                    rightPadding: 3 * dpiScale;
                    tooltip: qsTr("Set to current cursor position");
                    onClicked: repairEnd.value = Math.round(window.videoArea.vid.timestamp * 10) / 10;
                }
            }
        }
        Label {
            position: Label.LeftPosition;
            text: qsTr("Blend duration");
            inner.width: 80 * dpiScale;
            NumberField {
                id: repairBlend;
                unit: "ms";
                precision: 0;
                from: 0;
                to: 1000;
                width: 80 * dpiScale;
                value: 300;
                tooltip: qsTr("Transition duration at region boundaries for smooth blending");
            }
        }
        Label {
            position: Label.LeftPosition;
            text: qsTr("Blend curve");
            ComboBox {
                id: repairBlendMethod;
                model: ["Linear", "Smooth"];
                font.pixelSize: 12 * dpiScale;
                width: parent.width;
                currentIndex: 1;
                tooltip: qsTr("Blend curve at region boundaries: Linear = constant speed, Smooth = cosine S-curve");
            }
        }
        Label {
            position: Label.LeftPosition;
            text: qsTr("Blend bias");
            inner.width: 120 * dpiScale;
            NumberField {
                id: repairBlendBias;
                precision: 2;
                from: 0.01; to: 0.99;
                defaultValue: 0.5;
                font.pixelSize: 12 * dpiScale;
                width: parent.width;
                Component.onCompleted: value = 0.5;
                tooltip: qsTr("Position of the blend midpoint: 0.5 = symmetric, <0.5 = ease into repair, >0.5 = ease out of repair");
            }
        }
        Row {
            width: parent.width;
            spacing: 5 * dpiScale;
            ComboBox {
                id: repairOfMethod;
                model: ["AKAZE", "OpenCV (PyrLK)", "OpenCV (DIS)"];
                font.pixelSize: 12 * dpiScale;
                width: parent.width / 2 - 2.5 * dpiScale;
                currentIndex: 2;
                tooltip: qsTr("Optical flow method");
            }
            ComboBox {
                id: repairPoseMethod;
                model: ["findEssentialMat", "Almeida", "EightPoint", "findHomography"];
                font.pixelSize: 12 * dpiScale;
                width: parent.width / 2 - 2.5 * dpiScale;
                currentIndex: 0;
                tooltip: qsTr("Pose method");
            }
        }
        Label {
            position: Label.LeftPosition;
            text: qsTr("Resolution");
            inner.width: 80 * dpiScale;
            ComboBox {
                id: repairResolution;
                model: [QT_TRANSLATE_NOOP("Popup", "Full"), "4k", "1080p", "720p", "480p"];
                font.pixelSize: 12 * dpiScale;
                width: parent.width;
                currentIndex: 3;
                tooltip: qsTr("Processing resolution for optical flow analysis");
            }
        }
        Label {
            position: Label.LeftPosition;
            text: qsTr("Number of features");
            inner.width: 80 * dpiScale;
            NumberField {
                id: repairMaxFeatures;
                unit: "";
                precision: 0;
                from: 50;
                to: 2000;
                width: 80 * dpiScale;
                value: 50;
                tooltip: qsTr("Number of tracking points");
            }
        }
        Label {
            position: Label.LeftPosition;
            text: qsTr("Feature threshold");
            inner.width: 80 * dpiScale;
            NumberField {
                id: repairThreshold;
                unit: "";
                precision: 4;
                from: 0;
                to: 0.01;
                width: 80 * dpiScale;
                defaultValue: 0.0007;
                tooltip: qsTr("Feature detection sensitivity");
                Component.onCompleted: value = 0.0007;
            }
        }
        Button {
            text: controller.gyro_replace_in_progress ? qsTr("Analyzing... %1%").arg(Math.round(repairSection.gyroRepairProgress)) : qsTr("Analyze");
            iconName: "chart";
            width: parent.width;
            enabled: !controller.gyro_replace_in_progress && repairEnd.value > repairStart.value;
            property real gyroRepairProgress: 0;
            onClicked: {
                const start_us = Math.round(repairStart.value * 1000);
                const end_us = Math.round(repairEnd.value * 1000);
                const blend_us = Math.round(repairBlend.value * 1000);
                const proc_height = [-1, 2160, 1080, 720, 480][repairResolution.currentIndex];
                controller.start_gyro_replace(start_us, end_us, blend_us, repairBlendMethod.currentIndex, repairBlendBias.value, repairOfMethod.currentIndex, repairPoseMethod.currentIndex, repairMaxFeatures.value, repairThreshold.value, proc_height);
            }
        }
        Connections {
            target: controller;
            function onGyro_replace_progress(progress: real, ready: int, total: int): void {
                repairSection.gyroRepairProgress = progress * 100;
            }
            function onReplacement_regions_updated(): void {
                repairRegionsRepeater.model = [];
                repairRegionsRepeater.model = controller.replacement_regions_model;
            }
        }
        Repeater {
            id: repairRegionsRepeater;
            model: controller.replacement_regions_model;
            delegate: Rectangle {
                width: repairSection.width;
                height: 30 * dpiScale;
                color: model.index === controller.selected_repair_region ? repairSection.rowBgSelected : repairSection.rowBgUnselected;
                radius: 4 * dpiScale;
                border.width: model.index === controller.selected_repair_region ? 1 : 0;
                border.color: repairSection.regionBorder;
                CheckBox {
                    x: 4 * dpiScale;
                    y: (parent.height - height) / 2;
                    checked: model.enabled;
                    onClicked: controller.toggle_gyro_replace(model.index, checked);
                    tooltip: qsTr("Enable/disable repaired gyro data");
                }
                BasicText {
                    x: 30 * dpiScale;
                    y: (parent.height - height) / 2;
                    text: "%1 - %2 ms".arg((model.start_us / 1000).toFixed(1)).arg((model.end_us / 1000).toFixed(1));
                    font.pixelSize: 11 * dpiScale;
                    verticalAlignment: Text.AlignVCenter;
                    width: parent.width - 60 * dpiScale;
                    elide: Text.ElideRight;
                }
                Button {
                    text: "X";
                    x: parent.width - 26 * dpiScale;
                    y: (parent.height - height) / 2;
                    width: 22 * dpiScale;
                    height: 22 * dpiScale;
                    font.pixelSize: 11 * dpiScale;
                    leftPadding: 0;
                    rightPadding: 0;
                    tooltip: qsTr("Remove this repair");
                    onClicked: controller.remove_gyro_replace(model.index);
                }
                MouseArea {
                    anchors.fill: parent;
                    acceptedButtons: Qt.LeftButton;
                    propagateComposedEvents: true;
                    onClicked: (mouse) => {
                        controller.selected_repair_region = (controller.selected_repair_region === model.index) ? -1 : model.index;
                        controller.selected_repair_region_changed();
                        mouse.accepted = false;
                    }
                    onPressed: (mouse) => mouse.accepted = false;
                }
            }
        }
        Button {
            text: qsTr("Clear all repairs");
            iconName: "bin";
            width: parent.width;
            visible: controller.replacement_regions_model.rowCount() > 0;
            onClicked: controller.clear_gyro_replace();
        }
     }
}
