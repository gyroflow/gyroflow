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
    property alias advanced: advanced;
    property alias outputFile: outputFile.text;
    property alias sync: sync;
    property alias stab: stab;
    property alias renderBtn: renderBtn;

    readonly property bool wasModified: window.videoArea.vid.loaded;

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
            Column {
                width: parent.width;
                parent: leftPanel;
                id: gflogo;
                
                Item {
                    width: parent.width;
                    height: children[0].height * 1.5;
                    Image {
                        source: "qrc:/resources/logo" + (style === "dark"? "_white" : "_black") + ".svg"
                        sourceSize.width: Math.min(300 * dpiScale, parent.width * 0.9);
                        anchors.centerIn: parent;
                    }
                }
                Hr { }
            }

            Menu.VideoInformation {
                id: vidInfo;
                onSelectFileRequest: fileDialog.open();
            }
            Hr { }
            Menu.LensProfile {
                id: lensProfile;
            }
            Hr { }
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
                        anchors.verticalCenterOffset: -2 * dpiScale;
                        width: exportbar.width - parent.children[0].width - exportbar.children[2].width - 75 * dpiScale;

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
                    anchors.rightMargin: 55 * dpiScale;
                    anchors.verticalCenter: parent.verticalCenter;
                    text: isAddToQueue? (render_queue.editing_job_id > 0? qsTr("Save") : qsTr("Add to render queue")) : qsTr("Export");
                    icon.name: "video";
                    enabled: window.videoArea.vid.loaded && exportSettings.canExport && !videoArea.videoLoader.active;
                    opacity: enabled? 1.0 : 0.6;
                    popup.width: width * 2;
                    Ease on opacity { }
                    fadeWhenDisabled: false;
                    property bool isAddToQueue: false;

                    model: [isAddToQueue? QT_TRANSLATE_NOOP("Popup", "Export") : QT_TRANSLATE_NOOP("Popup", render_queue.editing_job_id > 0? "Save" : "Add to render queue"), QT_TRANSLATE_NOOP("Popup", "Export .gyroflow file (including gyro data)"), QT_TRANSLATE_NOOP("Popup", "Export .gyroflow file")];

                    function renameOutput() {
                        const orgOutput = outputFile.text;
                        let output = orgOutput;
                        let i = 1;
                        while (controller.file_exists(output)) {
                            output = orgOutput.replace(/_stabilized(_\d+)?\.([a-z0-9]+)$/i, "_stabilized_" + i++ + ".$2");
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
                            messageBox(Modal.Question, qsTr("Output file already exists, do you want to overwrite it?"), [
                                { text: qsTr("Yes"), clicked: () => { allowFile = true; renderBtn.render(); } },
                                { text: qsTr("Rename"), clicked: renameOutput },
                                { text: qsTr("No"), accent: true },
                            ]);
                            return;
                        }

                        videoArea.vid.grabToImage(function(result) {
                            const job_id = render_queue.add(controller, JSON.stringify(exportSettings.getExportOptions()), controller.image_to_b64(result.image));
                            if (renderBtn.isAddToQueue) {
                                // Add to queue
                                videoArea.queue.shown = true;
                            } else {
                                // Export now
                                render_queue.main_job_id = job_id;
                                render_queue.render_job(job_id, true);
                            }
                        }, Qt.size(50 * dpiScale * videoArea.vid.parent.ratio, 50 * dpiScale));
                    }
                    onClicked: {
                        allowFile = false;
                        allowLens = false;
                        allowSync = false;
                        window.videoArea.vid.pause();
                        render();
                    }
                    popup.onClicked: (index) => {
                        if (index == 0) { // Add to render queue or Export
                            renderBtn.isAddToQueue = !renderBtn.isAddToQueue;
                            popup.close();
                            renderBtn.clicked();
                            return;
                        }
                        controller.export_gyroflow_file(index == 2, JSON.stringify(exportSettings.getExportOptions()), "", false);
                    }
                }
                LinkButton {
                    anchors.right: parent.right;
                    anchors.rightMargin: 5 * dpiScale;
                    leftPadding: 10 * dpiScale;
                    rightPadding: 10 * dpiScale;
                    icon.width: 25 * dpiScale;
                    icon.height: 25 * dpiScale;
                    // textColor: styleTextColor;
                    anchors.verticalCenter: parent.verticalCenter;
                    icon.name: "queue";
                    tooltip: qsTr("Render queue");
                    onClicked: videoArea.queue.shown = !videoArea.queue.shown;
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
            Hr { }
            Menu.Stabilization {
                id: stab;
            }
            Hr { }
            Menu.Export {
                id: exportSettings;
            }
            Hr { }
            Menu.Advanced {
                id: advanced;
            }
        }
    }

    Shortcuts {
        videoArea: videoArea;
    }

    function messageBox(type: int, text: string, buttons: list, parent: QtObject, textFormat: int): Modal {
        if (typeof textFormat === "undefined") textFormat = Text.AutoText; // default
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
        function onError(text: string, arg: string, callback: string) {
            text = getReadableError(qsTr(text).arg(arg));
            messageBox(Modal.Error, text, [ { text: qsTr("Ok"), clicked: window[callback] } ]);
        }
        function onMessage(text: string, arg: string, callback: string) {
            messageBox(Modal.Info, qsTr(text).arg(arg), [ { text: qsTr("Ok"), clicked: window[callback] } ]);
        }
        function onRequest_recompute() {
            Qt.callLater(controller.recompute_threaded);
        }
        function onUpdates_available(version: string, changelog: string) {
            const heading = "<p align=\"center\">" + qsTr("There's a newer version available: %1.").arg("<b>" + version + "</b>") + "</p>\n\n";
            const el = messageBox(Modal.Info, heading + changelog, [ { text: qsTr("Download"),accent: true, clicked: () => Qt.openUrlExternally("https://github.com/gyroflow/gyroflow/releases") },{ text: qsTr("Close") }], undefined, Text.MarkdownText);
            el.t.horizontalAlignment = Text.AlignLeft;
        }
        function onRequest_location(path: string, thin: bool) {
            gfFileDialog.thin = thin;
            gfFileDialog.currentFolder = controller.path_to_url(path);
            gfFileDialog.open();
        }
        function onGyroflow_exists(path: string, thin: bool) {
            messageBox(Modal.Question, qsTr("`.gyroflow` file already exists, what do you want to do?"), [
                { text: qsTr("Overwrite"), "accent": true, clicked: () => {
                    controller.export_gyroflow_file(thin, exportSettings.getExportOptions(), path, true);
                } },
                { text: qsTr("Rename"), clicked: () => {
                    let output = path;
                    let i = 1;
                    while (controller.file_exists(output)) {
                        output = path.replace(/(_\d+)?\.([a-z0-9]+)$/i, "_" + i++ + ".$2");
                        if (i > 1000) break;
                    }
                    controller.export_gyroflow_file(thin, exportSettings.getExportOptions(), output, true);
                } },
                { text: qsTr("Choose a different location"), clicked: () => {
                    gfFileDialog.thin = thin;
                    gfFileDialog.currentFolder = controller.path_to_url(path);
                    gfFileDialog.open();
                } },
                { text: qsTr("Cancel") }
            ], undefined, Text.MarkdownText);
        }
    }
    FileDialog {
        id: gfFileDialog;
        fileMode: FileDialog.SaveFile;
        title: qsTr("Select file destination");
        nameFilters: ["*.gyroflow"];
        property bool thin: true;
        onAccepted: controller.export_gyroflow_file(thin, exportSettings.getExportOptions(), controller.url_to_path(selectedFile), true);
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

    function getReadableError(text: string): string {
        if (text.includes("ffmpeg")) {
            if (text.includes("Permission denied")) return qsTr("Permission denied. Unable to create or write file.\nChange the output path or run the program as administrator.\nMake sure you have write permissions to the target directory and make sure target file is not used by any other application.");
            if (text.includes("required nvenc API version")) return qsTr("NVIDIA GPU driver is too old, GPU encoding will not work for this format.\nUpdate your NVIDIA drivers to the newest version: %1.\nIf the issue is still present after driver update, your GPU probably doesn't support GPU encoding with this format. Disable GPU encoding in this case.").arg("<a href=\"https://www.nvidia.com/download/index.aspx\">https://www.nvidia.com/download/index.aspx</a>");
        
            text = text.replace(/ @ [A-F0-9]{6,}\]/g, "]"); // Remove ffmpeg function addresses

            // Remove duplicate lines
            text = [...new Set(text.split(/\r\n|\n\r|\n|\r/g))].join("\n");
        }
        if (text.startsWith("convert_format:")) {
            const format = text.split(":")[1].split(";")[0];
            return qsTr("GPU accelerated encoder doesn't support this pixel format (%1).\nDo you want to convert to a different supported pixel format or keep the original one and render on the CPU?").arg("<b>" + format + "</b>");
        }

        return text.trim();
    }

    function reportProgress(progress: real, type: string) {
        if (videoArea.videoLoader.active) {
            if (type === "loader") ui_tools.set_progress(progress);
            return;
        }
        ui_tools.set_progress(progress);
    }
}
