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
    property alias renderBtn: renderBtn;

    readonly property bool wasModified: window.videoArea.vid.loaded;

    function togglePlay() {
        window.videoArea.timeline.focus = true;
        const vid = window.videoArea.vid;
        if (vid.playing) vid.pause(); else vid.play();
    }

    FileDialog {
        id: fileDialog;
        property var extensions: [
            "mp4", "mov", "mxf", "mkv", "webm", "insv", "gyroflow", "png", "exr",
            "MP4", "MOV", "MXF", "MKV", "WEBM", "INSV", "GYROFLOW", "PNG", "EXR"
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

                        LinkButton {
                            anchors.right: parent.right;
                            height: parent.height - 1 * dpiScale;
                            text: "...";
                            font.underline: false;
                            font.pixelSize: 15 * dpiScale;
                            onClicked: {
                                outputFileDialog.defaultSuffix = outputFile.text.substring(outputFile.text.length - 3);
                                outputFileDialog.currentFile = controller.path_to_url(outputFile.text);
                                outputFileDialog.open();
                            }
                        }
                    }
                    FileDialog {
                        id: outputFileDialog;
                        fileMode: FileDialog.SaveFile;
                        title: qsTr("Select file destination");
                        nameFilters: Qt.platform.os == "android"? undefined : [qsTr("Video files") + " (*.mp4 *.mov *.png)"];
                        onAccepted: {
                            outputFile.text = controller.url_to_path(outputFileDialog.selectedFile);
                        }
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

                    function renameOutput() {
                        const orgOutput = outputFile.text;
                        let output = orgOutput;
                        let i = 1;
                        while (controller.file_exists(output)) {
                            output = orgOutput.replace(/_stabilized(_\d+)?\.mp4/, "_stabilized_" + i++ + ".mp4");
                            if (i > 1000) break;
                        }

                        outputFile.text = output;
                        render();
                    }
                    property bool allowFile: false;
                    property bool allowLens: false;
                    property bool allowSync: false;

                    function render() {
                        if (!controller.lens_loaded && !allowLens) {
                            messageBox(Modal.Warning, qsTr("Lens profile is not loaded, your result will be incorrect. Are you sure you want to render this file?"), [
                                { text: qsTr("Yes"), clicked: () => { allowLens = true; renderBtn.render(); }},
                                { text: qsTr("No"), accent: true },
                            ]);
                            return;
                        }
                        const usesQuats = window.motionData.hasQuaternions && window.motionData.integrationMethod === 0 && window.motionData.filename == window.vidInfo.filename;
                        if (!usesQuats && controller.offsets_model.rowCount() == 0 && !allowSync) {
                            messageBox(Modal.Warning, qsTr("There are no sync points present, your result will be incorrect. Are you sure you want to render this file?"), [
                                { text: qsTr("Yes"), clicked: () => { allowSync = true; renderBtn.render(); }},
                                { text: qsTr("No"), accent: true },
                            ]);
                            return;
                        }
                        if (controller.file_exists(outputFile.text) && !allowFile) {
                            messageBox(Modal.NoIcon, qsTr("Output file already exists, do you want to overwrite it?"), [
                                { text: qsTr("Yes"), clicked: () => { allowFile = true; renderBtn.render(); } },
                                { text: qsTr("Rename"), clicked: renameOutput },
                                { text: qsTr("No"), accent: true },
                            ]);
                            return;
                        }

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
                            exportSettings.outAudio,
                            exportSettings.overridePixelFormat
                        );
                    }
                    onClicked: {
                        allowFile = false;
                        allowLens = false;
                        allowSync = false;
                        exportSettings.overridePixelFormat = "";
                        render();
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

    function messageBox(type, text, buttons, parent, textFormat) {
        if (textFormat === undefined ) textFormat = Text.AutoText; // default
        const el = Qt.createComponent("components/Modal.qml").createObject(parent || window, { textFormat: textFormat, text: text, iconType: type});
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
            text = getReadableError(qsTr(text).arg(arg));
            messageBox(Modal.Error, text, [ { "text": qsTr("Ok"), clicked: window[callback] } ]);
        }
        function onMessage(text, arg, callback) {
            messageBox(Modal.Info, qsTr(text).arg(arg), [ { "text": qsTr("Ok"), clicked: window[callback] } ]);
        }
        function onRequest_recompute() {
            Qt.callLater(controller.recompute_threaded);
        }
        function onUpdates_available(version, changelog) {
            const heading = "<p align=\"center\">" + qsTr("There's a newer version available: %1.").arg("<b>" + version + "</b>") + "</p>\n\n";
            const el = messageBox(Modal.Info, heading + changelog, [ { text: qsTr("Download"),accent: true, clicked: () => Qt.openUrlExternally("https://github.com/gyroflow/gyroflow/releases") },{ text: qsTr("Close") }], undefined, Text.MarkdownText);
            el.t.horizontalAlignment = Text.AlignLeft;
        }
    }

    Component.onCompleted: {
        controller.check_updates();

        QT_TRANSLATE_NOOP("App", "An error occured: %1");
        QT_TRANSLATE_NOOP("App", "Gyroflow file exported to %1.");
        QT_TRANSLATE_NOOP("App", "--REPLACE_WITH_NATIVE_NAME_OF_YOUR_LANGUAGE_IN_YOUR_LANGUAGE--", "Translate this to the native name of your language");

        if (!isLandscape) {
            isLandscapeChanged();
        }
    }

    function getReadableError(text) {
        if (text.includes("ffmpeg")) {
            if (text.includes("Permission denied")) return qsTr("Permission denied. Unable to create or write file.\nChange the output path or run the program as administrator.\nMake sure you have write permissions to the target directory and make sure target file is not used by any other application.");
            if (text.includes("required nvenc API version")) return qsTr("NVIDIA GPU driver is too old, GPU encoding will not work for this format.\nUpdate your NVIDIA drivers to the newest version: %1.\nIf the issue is still present after driver update, your GPU probably doesn't support GPU encoding with this format. Disable GPU encoding in this case.").arg("<a href=\"https://www.nvidia.com/download/index.aspx\">https://www.nvidia.com/download/index.aspx</a>");
        }

        return text;
    }
}
