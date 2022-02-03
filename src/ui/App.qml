// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Window
import QtQuick.Controls as QQC
import QtQuick.Dialogs

import "."
import "components/"
import "menu/" as Menu

Rectangle {
    id: window;
    visible: true
    color: styleBackground;
    anchors.fill: parent;

    property QtObject controller: main_controller;

    property bool isLandscape: width > height;
    onIsLandscapeChanged: {
        if (isLandscape) {
            // Landscape layout
            leftPanel.y = 0;
            rightPanel.x = Qt.binding(() => leftPanel.width + videoAreaCol.width);
            rightPanel.y = 0;
            videoAreaCol.x = Qt.binding(() => leftPanel.width);
            videoAreaCol.width = Qt.binding(() => mainLayout.width - leftPanel.width - rightPanel.width);
            videoAreaCol.height = Qt.binding(() => mainLayout.height);
            leftPanel.fixedWidth = 0;
            rightPanel.fixedWidth = 0;
        } else {
            // Portrait layout
            videoAreaCol.y = 0;
            videoAreaCol.x = 0;
            videoAreaCol.width = Qt.binding(() => window.width);
            videoAreaCol.height = Qt.binding(() => window.height * 0.5);
            leftPanel.fixedWidth = Qt.binding(() => window.width * 0.4);
            rightPanel.fixedWidth = Qt.binding(() => window.width * 0.6);
            leftPanel.y = Qt.binding(() => videoAreaCol.height);
            rightPanel.x = Qt.binding(() => leftPanel.width);
            rightPanel.y = Qt.binding(() => videoAreaCol.height);
        }
    }
    property alias vidInfo: vidInfo;
    property alias videoArea: videoArea;
    property alias motionData: motionData;
    property alias lensProfile: lensProfile;
    property alias exportSettings: exportSettings;
    property alias outputFile: outputFile.text;
    property alias sync: sync;
    property alias stab: stab;

    FileDialog {
        id: fileDialog;
        property var extensions: [
            "mp4", "mov", "mxf", "mkv", "webm", "insv", "gyroflow",
            "MP4", "MOV", "MXF", "MKV", "WEBM", "INSV", "GYROFLOW"
        ];

        title: qsTr("Choose a video file")
        nameFilters: Qt.platform.os == "android"? undefined : [qsTr("Video files") + " (*." + extensions.join(" *.") + ")"];
        onAccepted: videoArea.loadFile(fileDialog.selectedFile);
    }

    Item {
        id: mainLayout;
        width: parent.width;
        height: parent.height - y;

        SidePanel {
            id: leftPanel;
            direction: SidePanel.HandleRight;
            topPadding: gflogo.height;
            Item {
                id: gflogo;
                parent: leftPanel;
                width: parent.width;
                height: children[0].height + 35 * dpiScale;
                Image {
                    source: "qrc:/resources/logo" + (style === "dark"? "_white" : "_black") + ".svg"
                    sourceSize.width: Math.min(300 * dpiScale, parent.width * 0.9);
                    anchors.centerIn: parent;
                }
                Hr { anchors.bottom: parent.bottom; }
            }

            Menu.VideoInformation {
                id: vidInfo;
                onSelectFileRequest: fileDialog.open();
            }
            Menu.LensProfile {
                id: lensProfile;
            }
            Menu.MotionData {
                id: motionData;
            }
        }

        Column {
            id: videoAreaCol;
            y: 0;
            x: leftPanel.width;
            width: parent? parent.width - leftPanel.width - rightPanel.width : 0;
            height: parent? parent.height : 0;
            VideoArea {
                id: videoArea;
                height: parent.height - exportbar.height;
                vidInfo: vidInfo;
            }

            // Bottom bar
            Rectangle {
                id: exportbar;
                width: parent.width;
                height: 60 * dpiScale;
                color: styleBackground2;

                Hr { width: parent.width; }

                Row {
                    height: parent.height;
                    spacing: 10 * dpiScale;
                    BasicText {
                        text: qsTr("Output path:");
                        anchors.verticalCenter: parent.verticalCenter;
                    }
                    TextField {
                        id: outputFile;
                        text: "";
                        anchors.verticalCenter: parent.verticalCenter;
                        width: exportbar.width - parent.children[0].width - exportbar.children[2].width - 30 * dpiScale;
                    }
                }

                SplitButton {
                    id: renderBtn;
                    accent: true;
                    anchors.right: parent.right;
                    anchors.rightMargin: 15 * dpiScale;
                    anchors.verticalCenter: parent.verticalCenter;
                    text: qsTr("Export");
                    icon.name: "video";
                    enabled: window.videoArea.vid.loaded && exportSettings.canExport && !videoArea.videoLoader.active;
                    opacity: enabled? 1.0 : 0.6;
                    popup.width: width * 2;
                    Ease on opacity { }
                    fadeWhenDisabled: false;

                    Component.onCompleted: {
                        QT_TRANSLATE_NOOP("Popup", "Add to render queue");
                    }

                    model: [QT_TRANSLATE_NOOP("Popup", "Export .gyroflow file (including gyro data)"), QT_TRANSLATE_NOOP("Popup", "Export .gyroflow file"), ];

                    function doRender() {
                        controller.render(
                            exportSettings.outCodec, 
                            exportSettings.outCodecOptions, 
                            outputFile.text, 
                            videoArea.trimStart, 
                            videoArea.trimEnd, 
                            exportSettings.outWidth, 
                            exportSettings.outHeight, 
                            exportSettings.outBitrate, 
                            exportSettings.outGpu, 
                            exportSettings.outAudio
                        );
                    }
                    function renameOutput() {
                        const orgOutput = outputFile.text;
                        let output = orgOutput;
                        let i = 1;
                        while (controller.file_exists(output)) {
                            output = orgOutput.replace(/_stabilized(_\d+)?\.mp4/, "_stabilized_" + i++ + ".mp4");
                            if (i > 1000) break;
                        }

                        outputFile.text = output;
                        clicked();
                    }
                    onClicked: {
                        if (controller.file_exists(outputFile.text)) {
                            messageBox(Modal.NoIcon, qsTr("Output file already exists, do you want to overwrite it?"), [
                                { text: qsTr("Yes"), clicked: doRender },
                                { text: qsTr("Rename"), clicked: renameOutput },
                                { text: qsTr("No"), accent: true },
                            ]);
                        } else {
                            doRender();
                        }
                    }
                    popup.onClicked: (index) => {
                        controller.export_gyroflow(index == 1);
                    }
                    
                    Connections {
                        target: controller;
                        function onRender_progress(progress, frame, total_frames, finished) {
                            videoArea.videoLoader.active = !finished;
                            videoArea.videoLoader.progress = videoArea.videoLoader.active? progress : -1;
                            videoArea.videoLoader.text = videoArea.videoLoader.active? qsTr("Rendering %1... %2").arg("<b>" + (progress * 100).toFixed(2) + "%</b>").arg("<font size=\"2\">(" + frame + "/" + total_frames + ")</font>") : "";
                            videoArea.videoLoader.cancelable = true;

                            function getFolder(v) {
                                let idx = v.lastIndexOf("/");
                                if (idx == -1) idx = v.lastIndexOf("\\");
                                if (idx == -1) return "";
                                return v.substring(0, idx + 1);
                            }

                            if (total_frames > 0 && finished) {
                                messageBox(Modal.Success, qsTr("Rendering completed. The file was written to: %1.").arg("<br><b>" + outputFile.text + "</b>"), [
                                    { text: qsTr("Open rendered file"), clicked: () => controller.open_file_externally(outputFile.text) },
                                    { text: qsTr("Open file location"), clicked: () => controller.open_file_externally(getFolder(outputFile.text)) },
                                    { text: qsTr("Ok") }
                                ]);
                            }
                        }
                    }
                }
            }
        }

        SidePanel {
            id: rightPanel;
            x: leftPanel.width + videoAreaCol.width;
            direction: SidePanel.HandleLeft;
            Menu.Synchronization {
                id: sync;
            }
            Menu.Stabilization {
                id: stab;
            }
            Menu.Export {
                id: exportSettings;
            }
            Menu.Advanced {

            }
        }
    }

    function messageBox(type, text, buttons, parent) {
        const el = Qt.createComponent("components/Modal.qml").createObject(parent || window, { text: text, iconType: type });
        el.onClicked.connect((index) => {
            if (buttons[index].clicked)
                buttons[index].clicked();
            el.opened = false;
            el.destroy(1000);
        });
        let buttonTexts = [];
        for (const i in buttons) {
            buttonTexts.push(buttons[i].text);
            if (buttons[i].accent) {
                el.accentButton = i;
            }
        }
        el.buttons = buttonTexts;
        
        el.opened = true;
        return el;
    }

    Connections {
        target: controller;
        function onError(text, arg, callback) {
            messageBox(Modal.Error, qsTr(text).arg(arg), [ { "text": qsTr("Ok"), clicked: window[callback] } ]);
        }
        function onMessage(text, arg, callback) {
            messageBox(Modal.Info, qsTr(text).arg(arg), [ { "text": qsTr("Ok"), clicked: window[callback] } ]);
        }
        function onRequest_recompute() {
            Qt.callLater(controller.recompute_threaded);
        }
        function onUpdates_available(version, changelog) {
            const body = changelog? "<p align=\"left\">" + changelog + "</p>" : "";
            const el = messageBox(Modal.Info, qsTr("There's a newer version available: %1.").arg("<b>" + version + "</b>") + body, [ { text: qsTr("Download"), accent: true, clicked: () => Qt.openUrlExternally("https://github.com/gyroflow/gyroflow/releases") }, { text: qsTr("Close") }])
            el.t.textFormat = Text.RichText;
        }
    }

    Component.onCompleted: {
        controller.check_updates();

        QT_TRANSLATE_NOOP("App", "An error occured: %1");
        QT_TRANSLATE_NOOP("App", "Gyroflow file exported to %1.");

        if (!isLandscape) {
            isLandscapeChanged();
        }
    }
}
