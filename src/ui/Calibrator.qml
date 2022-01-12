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
    width: 1000;
    height: 700;
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

    FileDialog {
        id: fileDialog;
        property var extensions: [
            "mp4", "mov", "mxf", "mkv", "webm", "insv", 
            "MP4", "MOV", "MXF", "MKV", "WEBM", "INSV"
        ];

        title: qsTr("Choose a video file")
        nameFilters: Qt.platform.os == "android"? undefined : [qsTr("Video files") + " (*." + extensions.join(" *.") + ")"];
        onAccepted: videoArea.loadFile(fileDialog.selectedFile);
    }

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
}
