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
    minimumWidth: 900 * dpiScale;
    minimumHeight: 400 * dpiScale;

    property bool isLandscape: width > height;
    onIsLandscapeChanged: {
        if (isLandscape) {
            // Landscape layout
            leftPanel.y = 0;
            videoAreaCol.x = Qt.binding(() => leftPanel.width);
            videoAreaCol.width = Qt.binding(() => mainLayout.width - videoAreaCol.x);
            videoAreaCol.height = Qt.binding(() => mainLayout.height);
            leftPanel.fixedWidth = 0;
        } else {
            // Portrait layout
            videoAreaCol.x = 0;
            videoAreaCol.width = Qt.binding(() => calibrator_window.width);
            videoAreaCol.height = Qt.binding(() => calibrator_window.height * 0.5);
            leftPanel.fixedWidth = Qt.binding(() => calibrator_window.width);
            leftPanel.y = Qt.binding(() => videoAreaCol.height);
        }
    }

    property QtObject controller: calib_controller;

    property alias videoArea: videoArea;
    property alias lensCalib: lensCalib;

    Material.theme: Material.Dark;
    Material.accent: Material.Blue;

    title: qsTr("Lens calibrator");

    Component.onCompleted: {
        ui_tools.set_icon(calibrator_window);
        if (!isMobile) {
            Qt.callLater(() => {
                width = width + 1;
                height = height;
            });
        } else {
            flags = Qt.WindowStaysOnTopHint;
            Qt.callLater(() => { calibrator_window.showFullScreen(); });
        }
    }

    function messageBox(type: int, text: string, buttons: list<var>, parent: QtObject, textFormat: int, identifier: string): Modal {
        return window.messageBox(type, text, buttons, parent || calibrator_window.contentItem, textFormat, identifier);
    }

    Connections {
        target: controller;
        function onError(text: string, arg: string, callback: string) {
            messageBox(Modal.Error, qsTr(text).arg(arg), [ { "text": qsTr("Ok"), clicked: window[callback] } ]);
        }
        function onRequest_recompute() {
            Qt.callLater(controller.recompute_threaded);
        }
        function onCalib_progress(progress: real, rms: real, ready: int, total: int, good: int, sharpness: real) {
            lensCalib.infoList.rms = rms;
            videoArea.videoLoader.active = progress < 1 || rms == 0;
            videoArea.videoLoader.additional = " - " + qsTr("%1 good frames").arg(good);
            videoArea.videoLoader.additionalLine = "<br>" + qsTr("Pattern sharpness: %1").arg(sharpness > 0? sharpness.toFixed(2) + "px" : "---");
            videoArea.videoLoader.currentFrame = ready;
            videoArea.videoLoader.totalFrames = total;
            videoArea.videoLoader.text = progress < 1? qsTr("Analyzing %1...") : "";
            videoArea.videoLoader.progress = progress < 1? progress : -1;
            videoArea.videoLoader.cancelable = true;
            if (!videoArea.videoLoader.active) {
                Qt.callLater(controller.recompute_threaded);
                let model = [];
                model[QT_TRANSLATE_NOOP("TableList", "Reprojection error")] = rms == 0? "---" : rms.toLocaleString(Qt.locale(), "f", 5);
                model[QT_TRANSLATE_NOOP("TableList", "Good frames")] = good;
                model[QT_TRANSLATE_NOOP("TableList", "Average pattern sharpness")] = sharpness.toLocaleString(Qt.locale(), "f", 2) + " px";
                lensCalib.infoList.model = model;
                if (rms > 5) {
                    window.play_sound("error");
                    if (good < 5 && sharpness < lensCalib.maxSharpness.value) {
                        const msg = sharpness > 0? qsTr("Some patterns were detected, but their average sharpness was <b>%1 px</b> and max limit is <b>%2 px</b>.").arg(sharpness.toLocaleString(Qt.locale(), "f", 2)).arg(lensCalib.maxSharpness.value.toLocaleString(Qt.locale(), "f", 2))
                                                 : qsTr("No calibration patterns were detected.")
                        messageBox(Modal.Warning, msg + "\n" + qsTr("Make sure your calibration footage is as sharp as possible:\n- Use high shutter speed\n- Use good lighting\n- Move the camera slowly\n- Avoid motion blur\n- Make sure the pattern stays in focus.\n\nYou can increase the sharpness limit in the Advanced section."), [
                            { text: qsTr("Ok") }
                        ]);
                    }
                } else if (rms > 0) {
                    window.play_sound("success");
                }
            }
        }
    }

    function loadFiles(files: list<url>) {
        if (files.length == 1)
            return loadFile(files[0]);

        const files2 = [...files];
        messageBox(Modal.NoIcon, qsTr("You selected multiple files. Do you want to process them automatically and export lens profiles?"), [
            { text: qsTr("Yes"), accent: true, clicked: () => {
                batch.queue = files2;
                batch.start();
             } },
            { text: qsTr("No") }
        ]);
    }
    function loadFile(file: url) {
        lensCalib.infoList.rms = 0;
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
        property var extensions: [ "mp4", "mov", "mxf", "mkv", "webm", "insv", "png", "jpg", "exr", "dng", "braw", "r3d" ];

        title: qsTr("Choose a video file")
        nameFilters: Qt.platform.os == "android"? undefined : [qsTr("Video files") + " (*." + extensions.concat(extensions.map(x => x.toUpperCase())).join(" *.") + ")"];
        type: "calib-video";

        onAccepted: {
            if (fileDialog.selectedFiles.length > 1) loadFiles(fileDialog.selectedFiles);
            else                                     loadFile(fileDialog.selectedFile);
        }
        fileMode: FileDialog.OpenFiles;
    }

    // --------- Batch processing ---------
    Item {
        id: batch;
        property var queue: [];
        property bool active: false;
        property url currentFile;
        function runIn(ms: int, cb) {
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
            function onTelemetry_loaded(is_main_video: bool, filename: string, camera: string, additional_data: var) {
                calibrator_window.anyFileLoaded = true;
                Qt.callLater(videoArea.timeline.updateDurations);

                if (!batch.active) return;
                batch.runIn(3000, function tryStart() {
                    if (window.isDialogOpened) return batch.runIn(2000, tryStart);
                    lensCalib.autoCalibBtn.clicked();
                });
            }
            function onCalib_progress(progress: real, rms: real, ready: int, total: int, good: int, sharpness: real) {
                if (!batch.active) return;
                if (ready > 0 && rms > 0) {
                    batch.runIn(2000, function() {
                        console.log('rms', rms);
                        if (rms < 5) {
                            const folder = filesystem.get_folder(batch.currentFile);
                            let filename = filesystem.filename_with_extension(filesystem.get_filename(batch.currentFile), "json");

                            if (lensCalib.calibrationInfo.camera_brand && lensCalib.calibrationInfo.camera_model && lensCalib.calibrationInfo.lens_model) {
                                filename = controller.export_lens_profile_filename(lensCalib.calibrationInfo);
                            }

                            let output = filename;
                            let i = 1;
                            while (filesystem.exists_in_folder(folder, output)) {
                                output = filename.replace(/(_\d+)?\.json/, "_" + i++ + ".json");
                                if (i > 2000) break;
                            }

                            window.getSaveFileUrl(folder, output, function(url) {
                                controller.export_lens_profile(url, lensCalib.calibrationInfo, lensCalib.uploadProfile.checked);
                            });
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

    Item {
        id: mainLayout;
        width: parent.width;
        height: parent.height - y;

        SidePanel {
            id: leftPanel;
            direction: SidePanel.HandleRight;
            implicitWidth: settings.value("calibPanelSize", defaultWidth);
            onWidthChanged: settings.setValue("calibPanelSize", width);

            Menu.VideoInformation {
                id: vidInfo;
                isCalibrator: true;
                onSelectFileRequest: fileDialog.open2();
                opened: false;
                objectName: "calibinfo";
            }
            Hr {

            }
            Menu.LensCalibrate {
                id: lensCalib;
            }
        }

        Column {
            id: videoAreaCol;
            x: leftPanel.width;
            width: mainLayout.width - x;
            height: mainLayout.height;
            VideoArea {
                id: videoArea;
                height: parent.height;
                vidInfo: vidInfo;
                isCalibrator: true;
                Component.onCompleted: videoArea.timeline.chart.setAxisVisible(8, false);

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
                        iconName: "chessboard"
                        onClicked: Qt.createComponent("CalibrationTarget.qml").createObject(calibrator_window).showMaximized();
                    }
                    LinkButton {
                        anchors.horizontalCenter: parent.horizontalCenter;
                        text: qsTr("How to calibrate lens?");
                        onClicked: Qt.openUrlExternally("https://docs.gyroflow.xyz/app/getting-started/lens-calibration")
                    }
                }
            }
        }
    }

    Shortcuts {
        videoArea: videoArea;
    }

    property bool anyFileLoaded: false;
    property bool closeConfirmationModal: false;
    onClosing: (close) => {
        if (anyFileLoaded && !closeConfirmationModal) {
            messageBox(Modal.NoIcon, qsTr("Are you sure you want to close the calibrator?"), [
                { text: qsTr("Yes"), accent: true, clicked: () => { calibrator_window.close(); } },
                { text: qsTr("No"), clicked: () => { calibrator_window.closeConfirmationModal = false; } },
            ]);
            close.accepted = false;
            closeConfirmationModal = true;
        }
    }

    WindowCloseButton { onClicked: calibrator_window.close(); }
}
