// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Window
import QtQuick.Controls.Material
import QtQuick.Dialogs
import Qt.labs.settings

import "."
import "components/"
import "menu/" as Menu

Window {
    id: calibrator_window;
    width: Math.min(Screen.width, 1000 * dpiScale);
    height: Math.min(Screen.height, 700 * dpiScale);
    visible: true;
    color: styleBackground;

    property QtObject controller: calib_controller;

    property alias videoArea: videoArea;
    property alias lensCalib: lensCalib;
    
    Material.theme: Material.Dark;
    Material.accent: Material.Blue;

    title: qsTr("Lens calibrator");

    Component.onCompleted: {
        ui_tools.set_icon(calibrator_window);
        Qt.callLater(() => {
            calibrator_window.width = calibrator_window.width + 1;
            calibrator_window.height = calibrator_window.height;
        });
    }

    function messageBox(type, text, buttons, parent) {
        window.messageBox(type, text, buttons, parent || calibrator_window.contentItem);
    }
    
    Connections {
        target: controller;
        function onError(text, arg, callback) {
            messageBox(Modal.Error, qsTr(text).arg(arg), [ { "text": qsTr("Ok"), clicked: window[callback] } ]);
        }
        function onRequest_recompute() {
            Qt.callLater(controller.recompute_threaded);
        }
    }

    function batchProcess() {
        batch.queue = [...fileDialog.selectedFiles];
        batch.start();
    }
    function loadFile(file) {
        lensCalib.rms = 0;
        controller.reset_player(videoArea.vid);
        Qt.callLater(() => {
            ui_tools.init_calibrator();
            Qt.callLater(() => {
                controller.init_player(videoArea.vid);
                Qt.callLater(() => {
                    videoArea.loadFile(file);
                });
            });
        });
    }
    
    FileDialog {
        id: fileDialog;
        property var extensions: [
            "mp4", "mov", "mxf", "mkv", "webm", "insv", 
            "MP4", "MOV", "MXF", "MKV", "WEBM", "INSV"
        ];

        title: qsTr("Choose a video file")
        nameFilters: Qt.platform.os == "android"? undefined : [qsTr("Video files") + " (*." + extensions.join(" *.") + ")"];

        onAccepted: {
            if (fileDialog.selectedFiles.length > 1) {
                messageBox(Modal.NoIcon, qsTr("You selected multiple files. Do you want to process them automatically and export lens profiles?"), [
                    { text: qsTr("Yes"), accent: true, clicked: batchProcess },
                    { text: qsTr("No") }
                ]);
            } else {
                loadFile(fileDialog.selectedFile);
            }
        }
        fileMode: FileDialog.OpenFiles;
    }

    // --------- Batch processing ---------
    Item {
        id: batch;
        property var queue: [];
        property bool active: false;
        property url currentFile;
        function runIn(ms, cb) {
            batchTimer.cb = cb;
            batchTimer.interval = ms;
            batchTimer.start();
        }
        function start() {
            if (queue.length) {
                active = true;
                batch.currentFile = batch.queue.shift();
                calibrator_window.loadFile(batch.currentFile);
            } else {
                active = false;
            }
        }
        Connections {
            target: controller;
            function onTelemetry_loaded(is_main_video, filename, camera, imu_orientation, contains_gyro, contains_quats, frame_readout_time, camera_id_json) {
                calibrator_window.anyFileLoaded = true;
                if (!batch.active) return;
                batch.runIn(2000, function() {
                    lensCalib.autoCalibBtn.clicked();
                })
            }
            function onCalib_progress(progress, rms, ready, total, good) {
                if (!batch.active) return;
                if (ready > 0 && rms > 0) {
                    batch.runIn(2000, function() {
                        console.log('rms', rms);
                        if (rms < 2) {
                            const pathParts = batch.currentFile.toString().split(".");
                            pathParts.pop();
                            const outputFilename = pathParts.join(".") + ".json";

                            let output = outputFilename;
                            let i = 1;
                            while (controller.file_exists(output)) {
                                output = outputFilename.replace(/(_\d+)?\.json/, "_" + i++ + ".json");
                                if (i > 2000) break;
                            }

                            controller.export_lens_profile(output, lensCalib.calibrationInfo, lensCalib.uploadProfile.checked);
                        }
                        batch.runIn(1000, function() { batch.start(); });
                    });
                }
            }
        }
        Timer {
            id: batchTimer;
            property var cb;
            onTriggered: cb();
        }
    }
    // --------- Batch processing ---------

    Row {
        id: mainLayout;
        width: parent.width;
        height: parent.height - y;

        SidePanel {
            id: leftPanel;
            direction: SidePanel.HandleRight;

            Menu.VideoInformation {
                id: vidInfo;
                isCalibrator: true;
                onSelectFileRequest: fileDialog.open();
                opened: false;
            }
            Menu.LensCalibrate {
                id: lensCalib;
            }
        }

        Column {
            id: videoAreaCol;
            width: parent? parent.width - leftPanel.width : 0;
            height: parent? parent.height : 0;
            VideoArea {
                id: videoArea;
                height: parent.height;
                vidInfo: vidInfo;
                isCalibrator: true;

                Column {
                    parent: videoArea.dropRect;
                    anchors.centerIn: parent;
                    anchors.verticalCenterOffset: 100 * dpiScale;
                    spacing: 10 * dpiScale;
                    BasicText {
                        anchors.horizontalCenter: parent.horizontalCenter;
                        text: qsTr("or");
                    }
                    Button {
                        text: qsTr("Open calibration target");
                        icon.name: "chessboard"
                        onClicked: Qt.createComponent("CalibrationTarget.qml").createObject(calibrator_window).showMaximized();
                    }
                    LinkButton {
                        anchors.horizontalCenter: parent.horizontalCenter;
                        text: qsTr("How to calibrate lens?");
                        onClicked: Qt.openUrlExternally("https://docs.gyroflow.xyz/guide/calibration/") 
                    }
                }
            }
        }
    }
    Connections {
        target: controller;
        function onCalib_progress(progress, rms, ready, total, good) {
            lensCalib.rms = rms;
            videoArea.videoLoader.active = progress < 1 || rms == 0;
            videoArea.videoLoader.progress = progress < 1? progress : -1;
            videoArea.videoLoader.text = progress < 1? qsTr("Analyzing %1... %2").arg("<b>" + (progress * 100).toFixed(2) + "%</b>").arg(`<font size="2">(${ready}/${total} - ` + qsTr("%1 good frames").arg(good) + ")</font>") : "";
            videoArea.videoLoader.cancelable = true;
            if (!videoArea.videoLoader.active) {
                Qt.callLater(controller.recompute_threaded);
            }
        }
    }

    property bool anyFileLoaded: false;
    property bool closeConfirmationModal: false;
    onClosing: (close) => {
        if (anyFileLoaded && !closeConfirmationModal) {
            messageBox(Modal.NoIcon, qsTr("Are you sure you want to close the calibrator?"), [
                { text: qsTr("Yes"), accent: true, clicked: () => calibrator_window.close() },
                { text: qsTr("No"), clicked: () => calibrator_window.closeConfirmationModal = false },
            ]);
            close.accepted = false;
            closeConfirmationModal = true;
        }
    }
}
