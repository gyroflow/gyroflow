// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Window
import QtQuick.Controls as QQC
import QtQuick.Dialogs
import Qt.labs.settings

import "."
import "components/"
import "menu/" as Menu
import "Util.js" as Util

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
            rightPanel.x = Qt.binding(() => (window.isMobileLayout? 0 : leftPanel.width) + videoAreaCol.width);
            rightPanel.y = 0;
            videoAreaCol.x = Qt.binding(() => (videoArea.fullScreen || window.isMobileLayout? 0 : leftPanel.width));
            videoAreaCol.width = Qt.binding(() => mainLayout.width - (videoArea.fullScreen? 0 : (window.isMobileLayout? 0 : leftPanel.width) + rightPanel.width));
            videoAreaCol.height = Qt.binding(() => mainLayout.height);
            leftPanel.fixedWidth = 0;
            rightPanel.fixedWidth = 0;
        } else {
            // Portrait layout
            videoAreaCol.y = 0;
            videoAreaCol.x = 0;
            videoAreaCol.width = Qt.binding(() => window.width);
            videoAreaCol.height = Qt.binding(() => window.height * (videoArea.fullScreen? 1 : (window.isMobileLayout? (window.videoArea.vid.loaded && window.videoArea.vid.height > window.videoArea.vid.width? 0.6 : 0.4) : 0.5)));
            leftPanel.fixedWidth = Qt.binding(() => window.width * 0.4);
            rightPanel.fixedWidth = Qt.binding(() => window.width * (window.isMobileLayout? 1.0 : 0.6));
            leftPanel.y = Qt.binding(() => videoAreaCol.height);
            rightPanel.x = Qt.binding(() => window.isMobileLayout? 0 : leftPanel.width);
            rightPanel.y = Qt.binding(() => videoAreaCol.height);
        }
    }
    // property bool isMobileLayout: width < (1500 * dpiScale);
    property bool isMobileLayout: ((isMobile && screenSize < 7.0) || forceMobileLayout) && !forceDesktopLayout;
    onIsMobileLayoutChanged: {
        if (isMobileLayout) {
            vidInfo      .parent = inputsTab.inner;
            vidInfoHr    .parent = inputsTab.inner;
            lensProfile  .parent = inputsTab.inner;
            lensProfileHr.parent = inputsTab.inner;
            motionData   .parent = inputsTab.inner;

            sync    .parent = paramsTab.inner;
            syncHr  .parent = paramsTab.inner;
            stab    .parent = paramsTab.inner;
            stabHr  .parent = paramsTab.inner;
            advanced.parent = paramsTab.inner;

            outputPathLabel.parent = exportTab.inner;
            renderBtnRow   .parent = exportTab.inner;
            exportSettings .parent = exportTab.inner;
        } else {
            vidInfo      .parent = leftPanel.col;
            vidInfoHr    .parent = leftPanel.col;
            lensProfile  .parent = leftPanel.col;
            lensProfileHr.parent = leftPanel.col;
            motionData   .parent = leftPanel.col;

            sync          .parent = rightPanel.col;
            syncHr        .parent = rightPanel.col;
            stab          .parent = rightPanel.col;
            stabHr        .parent = rightPanel.col;
            exportSettings.parent = rightPanel.col;
            exportHr      .parent = rightPanel.col;
            advanced      .parent = rightPanel.col;

            outputPathLabel.parent = exportbar;
            renderBtnRow   .parent = exportbar;
        }
    }
    property alias vidInfo: vidInfo.item;
    property alias videoArea: videoArea;
    property alias motionData: motionData.item;
    property alias lensProfile: lensProfile.item;
    property alias outputFile: outputFile;
    property alias sync: sync.item;
    property alias stab: stab.item;
    property alias exportSettings: exportSettings.item;
    property alias advanced: advanced.item;
    property alias renderBtn: renderBtn;
    property alias settings: settings;

    readonly property bool wasModified: window.videoArea.vid.loaded;
    property bool isDialogOpened: false;

    Settings { id: settings; }

    FileDialog {
        id: fileDialog;
        property var extensions: [ "mp4", "mov", "mxf", "mkv", "webm", "insv", "gyroflow", "png", "jpg", "exr", "dng", "braw", "r3d" ];

        title: qsTr("Choose a video file")
        nameFilters: Qt.platform.os == "android"? undefined : [qsTr("Video files") + " (*." + extensions.concat(extensions.map(x => x.toUpperCase())).join(" *.") + ")"];
        type: "video";
        fileMode: FileDialog.OpenFiles;
        onAccepted: videoArea.loadMultipleFiles(selectedFiles, false);
    }

    property url pendingOpenFileOrg: openFileOnStart;
    property url pendingOpenFile: pendingOpenFileOrg;
    onPendingOpenFileOrgChanged: { pendingOpenFile = pendingOpenFileOrg; onItemLoaded(); }
    Connections {
        target: filesystem;
        function onUrl_opened(url: url) { pendingOpenFileOrg = ""; pendingOpenFileOrg = url; }
    }
    function onItemLoaded() {
        if (window.vidInfo && window.stab && window.exportSettings && window.sync && window.motionData && pendingOpenFile.toString()) {
            pendingFileLoadTimer.start();
        }
        tabs.updateHeights();
    }
    Timer {
        id: pendingFileLoadTimer;
        interval: 250;
        running: false;
        onTriggered: {
            if (pendingOpenFile.toString()) {
                videoArea.loadFile(pendingOpenFile);
                pendingOpenFile = "";
            }
        }
    }

    Item {
        id: mainLayout;
        width: parent.width;
        height: parent.height - y;

        SidePanel {
            id: leftPanel;
            direction: SidePanel.HandleRight;
            topPadding: gflogo.height;
            visible: !videoArea.fullScreen && !isMobileLayout;
            maxWidth: parent.width - rightPanel.width - 50 * dpiScale;
            implicitWidth: settings.value("leftPanelSize", defaultWidth);
            onWidthChanged: settings.setValue("leftPanelSize", width);
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

            ItemLoader { id: vidInfo; sourceComponent: Component {
                Menu.VideoInformation {
                    onSelectFileRequest: fileDialog.open2();
                }
            } }
            Hr { id: vidInfoHr; }
            ItemLoader { id: lensProfile; sourceComponent: Component {
                Menu.LensProfile { }
            } }
            Hr { id: lensProfileHr; }
            ItemLoader { id: motionData; sourceComponent: Component {
                Menu.MotionData { }
            } }
        }

        Column {
            id: videoAreaCol;
            y: 0;
            x: videoArea.fullScreen? 0 : leftPanel.width;
            width: parent? parent.width - (videoArea.fullScreen? 0 : leftPanel.width + rightPanel.width) : 0;
            height: parent? parent.height : 0;
            VideoArea {
                id: videoArea;
                height: parent.height - (videoArea.fullScreen || isMobileLayout? 0 : exportbar.height);
                vidInfo: vidInfo.item;
            }

            // Bottom bar
            Rectangle {
                id: exportbar;
                width: parent.width;
                height: 60 * dpiScale;
                color: styleBackground2;
                visible: !isMobileLayout;

                Hr { width: parent.width; }

                Label {
                    x: 10 * dpiScale;
                    id: outputPathLabel;
                    anchors.verticalCenter: (isMobileLayout? undefined : parent.verticalCenter);
                    anchors.verticalCenterOffset: -1 * dpiScale;
                    text: qsTr("Output path:");
                    position: isMobileLayout? Label.TopPosition : Label.LeftPosition;
                    width: parent.width - (isMobileLayout? 0 : renderBtnRow.width + 10 * dpiScale) - 2*x;
                    OutputPathField {
                        id: outputFile;
                        onFolderUrlChanged: {
                            if (exportSettings.item.preserveOutputPath.checked) {
                                const outputFolder = folderUrl.toString();
                                if (outputFolder) settings.setValue("preservedOutputPath", outputFolder);
                            }
                        }
                    }
                }

                Row {
                    id: renderBtnRow;
                    anchors.right: (isMobileLayout? undefined : parent.right);
                    anchors.rightMargin: 5 * dpiScale;
                    spacing: 5 * dpiScale;
                    anchors.verticalCenter: (isMobileLayout? undefined : parent.verticalCenter);
                    anchors.horizontalCenter: (isMobileLayout? parent.horizontalCenter : undefined);
                    anchors.horizontalCenterOffset: (queueBtn.width + spacing) / 2;
                    SplitButton {
                        id: renderBtn;
                        btn.accent: true;
                        text: {
                            if (addQueueDelayed) {
                                return qsTr("Added to queue");
                            } else if (isAddToQueue) {
                                return render_queue.editing_job_id > 0? qsTr("Save") : qsTr("Add to render queue");
                            } else {
                                return qsTr("Export");
                            }
                        }
                        iconName: addQueueDelayed ? "confirmed" : "video";
                        isDown: isMobileLayout;
                        property bool isAddToQueue: false;
                        property bool allowFile: false;
                        property bool allowLens: false;
                        property bool allowSync: false;
                        onIsAddToQueueChanged: updateModel();
                        enabled: window.videoArea.vid.loaded && outputFile.filename.length > 3;

                        property bool enabled2: window.videoArea.vid.loaded && exportSettings.item && exportSettings.item.canExport && !videoArea.videoLoader.active;
                        onEnabled2Changed: et.start();
                        Timer { id: et; interval: 200; onTriggered: renderBtn.btn.enabled = renderBtn.enabled2; }

                        property bool addQueueDelayed: false;
                        Timer {
                            id: delayAddQueue;
                            interval: 2000;
                            onTriggered:  {
                                renderBtn.addQueueDelayed = false;
                                renderBtn.btn.enabled = renderBtn.enabled2;
                            }
                        }

                        function updateModel() {
                            let m = [
                                ["export",        isAddToQueue? QT_TRANSLATE_NOOP("Popup", "Export") : (render_queue.editing_job_id > 0? QT_TRANSLATE_NOOP("Popup", "Save") : QT_TRANSLATE_NOOP("Popup", "Add to render queue"))],
                                ["create_preset", QT_TRANSLATE_NOOP("Popup", "Create settings preset")],
                                ["apply_all",     QT_TRANSLATE_NOOP("Popup", "Apply selected settings to all items in the render queue")],
                                ["export_proj:WithProcessedData", QT_TRANSLATE_NOOP("Popup", "Export project file (including processed gyro data)")],
                                ["export_proj:WithGyroData",      QT_TRANSLATE_NOOP("Popup", "Export project file (including gyro data)")],
                                ["export_proj:Simple",            QT_TRANSLATE_NOOP("Popup", "Export project file")]
                            ];
                            if (controller.project_file_url) m.push(["save", QT_TRANSLATE_NOOP("Popup", "Save project file")]);
                            model   = m.map(x => x[1]);
                            actions = m.map(x => x[0]);
                        }

                        model: [];
                        property list<string> actions: [];

                        Connections {
                            target: controller;
                            function onProject_file_url_changed() { renderBtn.updateModel(); }
                        }
                        Connections {
                            target: render_queue;
                            function onQueue_changed() { renderBtn.updateModel(); }
                        }
                        Component.onCompleted: updateModel();

                        function render() {
                            const fname = vidInfo.item.filename.toLowerCase();
                            if (fname.endsWith('.braw') || (fname.endsWith('.r3d') && !controller.find_redline()) || fname.endsWith('.dng')) {
                                messageBox(Modal.Info, qsTr("This format is not available for rendering.\nThe recommended workflow is to export project file and use one of [video editor plugins] (%1).").replace(/\[(.*?)\]/, '<a href="https://gyroflow.xyz/download#plugins"><font color="' + styleTextColor + '">$1</font></a>').arg("DaVinci Resolve, Final Cut Pro"), [
                                    { text: qsTr("Ok"), accent: true }
                                ]);
                                return;
                            }
                            if (!controller.lens_loaded && !allowLens) {
                                messageBox(Modal.Warning, qsTr("Lens profile is not loaded, your result will be incorrect. Are you sure you want to render this file?"), [
                                    { text: qsTr("Yes"), clicked: () => { allowLens = true; renderBtn.render(); }},
                                    { text: qsTr("No"), accent: true },
                                ]);
                                return;
                            }
                            const usesQuats = ((motionData.item.hasQuaternions && motionData.item.integrationMethod === 0) || motionData.item.hasAccurateTimestamps) && motionData.item.filename == vidInfo.item.filename;
                            if (!usesQuats && controller.offsets_model.rowCount() == 0 && !allowSync) {
                                messageBox(Modal.Warning, qsTr("There are no sync points present, your result will be incorrect. Are you sure you want to render this file?"), [
                                    { text: qsTr("Yes"), clicked: () => { allowSync = true; renderBtn.render(); }},
                                    { text: qsTr("No"), accent: true },
                                ]);
                                return;
                            }
                            const exists = filesystem.exists_in_folder(outputFile.folderUrl, outputFile.filename.replace("_%05d", "_00001"));
                            if ((exists || render_queue.file_exists_in_folder(outputFile.folderUrl, outputFile.filename)) && !allowFile) {
                                function overwrite() {
                                    allowFile = true;
                                    renderBtn.render();
                                }
                                function rename() {
                                    outputFile.setFilename(window.renameOutput(outputFile.filename, outputFile.folderUrl));
                                    renderBtn.render();
                                }

                                if (renderBtn.isAddToQueue && render_queue.overwrite_mode === 1) {
                                    overwrite();
                                    showNotification(Modal.Info, qsTr("Added to queue") + ", " + qsTr("file %1 will be overwritten").arg(outputFile.filename))
                                } else if (renderBtn.isAddToQueue && render_queue.overwrite_mode === 2) {
                                    rename();
                                    showNotification(Modal.Info, qsTr("Added to queue") + ", " + qsTr("file will be rendered to %1").arg(outputFile.filename))
                                } else {
                                    messageBox(Modal.Question, qsTr("Output file already exists, do you want to overwrite it?"), [
                                        { text: qsTr("Yes"), clicked: overwrite },
                                        { text: qsTr("Rename"), clicked: rename },
                                        { text: qsTr("No"), accent: true },
                                    ]);
                                }

                                return;
                            }

                            if (fname.endsWith('.r3d') && controller.find_redline()) {
                                messageBox(Modal.Info, "Gyroflow will use REDline to convert .R3D to ProRes before stabilizing in order to export from Gyroflow directly.\nIf you want to work on RAW data instead, export project file (Ctrl+S) and use one of [video editor plugins] (%1).".replace(/\[(.*?)\]/, '<a href="https://gyroflow.xyz/download#plugins"><font color="' + styleTextColor + '">$1</font></a>').arg("DaVinci Resolve, Final Cut Pro"), [
                                    { text: qsTr("Ok"), accent: true }
                                ], undefined, Text.StyledText, "r3d-conversion" );
                            }

                            const encoder = render_queue.get_default_encoder(window.exportSettings.outCodec, window.exportSettings.outGpu);
                            if ((encoder + "").endsWith("_amf") && window.exportSettings.outBitrate > 100) {
                                messageBox(Modal.Info, qsTr("Some AMD GPU encoders have a bug where it limits the bitrate to 20 Mbps, if the target bitrate is greater than 100 Mbps.\n\n" +
                                                            "Please check the file bitrate after rendering and if you're affected by this bug, you can either:\n" +
                                                            "- Set output bitrate to less than 100 Mbps\n" +
                                                            "- Use \"Custom encoder options\": `-rc cqp -qp_i 28 -qp_p 28`"), [
                                    { text: qsTr("Ok") },
                                ], undefined, Text.MarkdownText, "amd-bitrate-warning");
                            }

                            videoArea.vid.grabToImage(function(result) {
                                if (isSandboxed && (!outputFile.folderUrl.toString() || !filesystem.can_create_file(outputFile.folderUrl, outputFile.filename))) {
                                    let el = messageBox(Modal.Info, qsTr("Due to file access restrictions, you need to select the destination folder manually.\nClick Ok and select the destination folder."), [
                                        { text: qsTr("Ok"), clicked: () => {
                                            outputFile.selectFolder(outputFile.folderUrl, function(_) { renderBtn.btn.clicked(); });
                                        }},
                                    ], undefined, Text.AutoText, "file-access-restriction");
                                    if (!el) { // Don't show again triggered
                                        outputFile.selectFolder(outputFile.folderUrl, function(_) { renderBtn.btn.clicked(); });
                                    }
                                    return;
                                }
                                if (isMobile) {
                                    messageBox(Modal.Info, qsTr("Keep this app in the foreground and don't lock the screen.\nDue to limitations of the system video encoders, rendering in the background is not supported."), [
                                        { text: qsTr("Ok") },
                                    ], undefined, Text.AutoText, "keep-in-foreground");
                                }

                                const job_id = render_queue.add(window.getAdditionalProjectDataJson(), controller.image_to_b64(result.image));
                                if (renderBtn.isAddToQueue) {
                                    // Add to queue
                                    renderBtn.addQueueDelayed = true;
                                    renderBtn.btn.enabled = false;
                                    delayAddQueue.start();

                                    if (+settings.value("showQueueWhenAdding", "1"))
                                        videoArea.queue.shown = true;
                                } else {
                                    // Export now
                                    render_queue.main_job_id = job_id;
                                    render_queue.render_job(job_id);
                                }
                            }, Qt.size(50 * dpiScale * videoArea.vid.parent.ratio, 50 * dpiScale));
                        }
                        btn.onClicked: {
                            allowFile = false;
                            allowLens = false;
                            allowSync = false;
                            window.videoArea.vid.pause();
                            render();
                        }
                        popup.onClicked: (index) => {
                            const action = actions[index];
                            switch (action) {
                                case "export": // Add to render queue or Export
                                    renderBtn.isAddToQueue = !renderBtn.isAddToQueue;
                                    popup.close();
                                    renderBtn.btn.clicked();
                                break;
                                case "create_preset": // Create preset
                                case "apply_all": // Apply settings to render queue
                                    const el = Qt.createComponent("SettingsSelector.qml").createObject(window, { isPreset: index == 1 });
                                    el.opened = true;
                                    el.onApply.connect((obj) => {
                                        const allData = JSON.parse(controller.export_gyroflow_data("Simple", window.getAdditionalProjectData()));
                                        const finalData = el.getFilteredObject(allData, obj);

                                        if (finalData.hasOwnProperty("output")) {
                                            finalData.output.output_filename = ""; // Don't modify filenames, only target folder
                                        }
                                        if (obj.synchronization && obj.synchronization.do_autosync) {
                                            finalData.synchronization.do_autosync = true;
                                        }
                                        if (action == "create_preset") { // Preset
                                            presetFileDialog.presetData = finalData;
                                            presetFileDialog.open2();
                                        } else { // Apply
                                            render_queue.apply_to_all(JSON.stringify(finalData), window.getAdditionalProjectDataJson(), 0);
                                        }
                                    });
                                break;
                                case "export_proj:WithProcessedData":
                                case "export_proj:WithGyroData":
                                case "export_proj:Simple":
                                    window.saveProject(action.substring(12));
                                break;
                                case "save": window.saveProject(""); break;
                            }
                        }
                    }
                    LinkButton {
                        id: queueBtn;
                        leftPadding: 10 * dpiScale;
                        rightPadding: 10 * dpiScale;
                        icon.width: 25 * dpiScale;
                        icon.height: 25 * dpiScale;
                        // textColor: styleTextColor;
                        anchors.verticalCenter: parent.verticalCenter;
                        iconName: "queue";
                        tooltip: qsTr("Render queue");
                        onClicked: videoArea.queue.shown = !videoArea.queue.shown;
                    }
                }
            }
        }

        SidePanel {
            id: rightPanel;
            visible: !videoArea.fullScreen;
            x: leftPanel.width + videoAreaCol.width;
            direction: SidePanel.HandleLeft;
            maxWidth: parent.width - leftPanel.width - 50 * dpiScale;
            implicitWidth: settings.value("rightPanelSize", defaultWidth);
            onWidthChanged: settings.setValue("rightPanelSize", width);
            col.visible: !isMobileLayout;

            Tabs {
                id: tabs;
                Component.onCompleted: { parent = rightPanel; currentIndex = 0; }
                visible: isMobileLayout;
                tabs: [QT_TRANSLATE_NOOP("Tabs", "Inputs"), QT_TRANSLATE_NOOP("Tabs", "Parameters"), QT_TRANSLATE_NOOP("Tabs", "Export")];
                tabsIcons: ["video", "settings", "save"];
                tabsIconsSize: [20, 24, 24];

                TabColumn { id: inputsTab; parentHeight: rightPanel.height; }
                TabColumn { id: paramsTab; parentHeight: rightPanel.height; }
                TabColumn { id: exportTab; parentHeight: rightPanel.height; inner.spacing: 10 * dpiScale; }
            }

            ItemLoader { id: sync; sourceComponent: Component { Menu.Synchronization { } } }
            Hr { id: syncHr; }
            ItemLoader { id: stab; sourceComponent: Component { Menu.Stabilization { } } }
            Hr { id: stabHr; }
            ItemLoader { id: exportSettings; sourceComponent: Component { Menu.Export { showBtn: !window.isMobileLayout; } } }
            Hr { id: exportHr; visible: !isMobileLayout; }
            ItemLoader { id: advanced; sourceComponent: Component { Menu.Advanced { } } }
        }
    }

    Shortcuts {
        videoArea: videoArea;
    }

    function showNotification(type: int, text: string, textFormat: int) {
        if (typeof textFormat === "undefined") textFormat = Text.AutoText; // default
        const im = Qt.createComponent("components/InfoMessage.qml").createObject(window.videoArea.infoMessages, {
            text: text,
            type: type - 1,
            opacity: 0
        });
        im.t.textFormat = textFormat;
        im.opacity = 1;
        Qt.createQmlObject("import QtQuick; Timer { interval: 5000; running: true; }", im, "t1").onTriggered.connect(() => {
            im.opacity = 0;
            im.height = -5 * dpiScale;
            im.destroy(700);
        });
    }

    function messageBox(type: int, text: string, buttons: list<var>, parent: QtObject, textFormat: int, identifier: string): Modal {
        if (typeof textFormat === "undefined") textFormat = Text.AutoText; // default

        let el = null;

        if (identifier && +window.settings.value("dontShowAgain-" + identifier, 0)) {
            const clickedButton = +window.settings.value("dontShowAgain-" + identifier, 0) - 1;
            if (buttons.length == 1) {
                showNotification(type, text, textFormat);
                return null;
            } else {
                console.log("previously clicked", clickedButton);
                if (clickedButton != buttons.length - 1) { // Don't auto-click the last button (it's always Cancel/Close)
                    Qt.callLater(function() {
                        if (el)
                            el.clicked(clickedButton, true);
                    });
                }
            }
        }
        if (type == Modal.Error)   play_sound("error");
        if (type == Modal.Success) play_sound("success");

        el = Qt.createComponent("components/Modal.qml").createObject(parent || window, { textFormat: textFormat, iconType: type, modalIdentifier: identifier || "" });
        el.text = text;
        el.onClicked.connect((index, dontShowAgain) => {
            if (identifier && dontShowAgain) {
                window.settings.setValue("dontShowAgain-" + identifier, index + 1);
            }

            let returnVal = undefined;
            if (buttons[index].clicked)
                returnVal = buttons[index].clicked();
            if (returnVal !== false) {
                el.close();
                window.isDialogOpened = false;
            }
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
        window.isDialogOpened = true;
        return el;
    }
    function play_sound(type: string) {
        if (settings.value("playSounds", "true") == "true")
            controller.play_sound(type);
    }

    Connections {
        target: controller;
        function onError(text: string, arg: string, callback: string) {
            text = getReadableError(qsTr(text).arg(arg));
            if (text)
                messageBox(Modal.Error, text, [ { text: qsTr("Ok"), clicked: window[callback] } ]);
        }
        function onMessage(text: string, arg: string, callback: string, id: string) {
            messageBox(Modal.Info, qsTr(text).arg(arg), [ { text: qsTr("Ok"), clicked: window[callback] } ], null, undefined, id);
        }
        function onRequest_recompute() {
            Qt.callLater(controller.recompute_threaded);
        }
        function openUpdatePage() {
            if (Qt.platform.os == "android") {
                Qt.openUrlExternally("https://play.google.com/store/apps/details?id=xyz.gyroflow");
            } else if (Qt.platform.os == "ios") {
                Qt.openUrlExternally("https://apps.apple.com/us/app/gyroflow/id6447994244");
            } else if (Qt.platform.os == "macos" && isStorePackage) {
                Qt.openUrlExternally("https://apps.apple.com/us/app/gyroflow/id6447994244");
            } else if (Qt.platform.os == "windows" && isStorePackage) {
                // https://apps.microsoft.com/store/detail/gyroflow/9NZG7T0JCG9H
                Qt.openUrlExternally("ms-windows-store://pdp/?ProductId=9NZG7T0JCG9H");
            } else {
                Qt.openUrlExternally("https://github.com/gyroflow/gyroflow/releases");
            }
        }
        function onUpdates_available(version: string, changelog: string) {
            const heading = "<p align=\"center\">" + qsTr("There's a newer version available: %1.").arg("<b>" + version + "</b>") + "</p>\n\n";
            const el = messageBox(Modal.Info, heading + changelog, [ { text: qsTr("Download"),accent: true, clicked: () => openUpdatePage() },{ text: qsTr("Close") }], undefined, Text.MarkdownText);
            el.t.horizontalAlignment = Text.AlignLeft;
        }
        function onRequest_location(url: string, type: string) {
            gfFileDialog.projectType = type;
            gfFileDialog.currentFolder = filesystem.get_folder(url);
            gfFileDialog.open();
        }
    }
    FileDialog {
        id: gfFileDialog;
        fileMode: FileDialog.SaveFile;
        title: qsTr("Select file destination");
        nameFilters: ["*.gyroflow"];
        type: "output-project";
        property string projectType: "Simple";
        onAccepted: saveProjectToUrl(selectedFile, projectType);
    }
    FileDialog {
        id: presetFileDialog;
        fileMode: FileDialog.SaveFile;
        title: qsTr("Select file destination");
        nameFilters: ["*.gyroflow"];
        type: "output-preset";
        property var presetData: ({});
        onAccepted: controller.export_preset(selectedFile, presetData);
    }

    Component.onCompleted: {
        controller.check_updates();

        QT_TRANSLATE_NOOP("App", "An error occured: %1");
        QT_TRANSLATE_NOOP("App", "Gyroflow file exported to %1.");
        QT_TRANSLATE_NOOP("App", "--REPLACE_WITH_NATIVE_NAME_OF_YOUR_LANGUAGE_IN_YOUR_LANGUAGE--", "Translate this to the native name of your language");
        QT_TRANSLATE_NOOP("App", "Gyroflow will shut down the computer in 60 seconds because all tasks have been completed.");
        QT_TRANSLATE_NOOP("App", "Gyroflow will reboot the computer in 60 seconds because all tasks have been completed.");

        Qt.callLater(filesystem.restore_allowed_folders);
    }

    function getReadableError(text: string): string {
        if (text.includes("ffmpeg")) {
            if (text.includes("Encoder not found") && text.includes("libx26") && controller.check_external_sdk("ffmpeg_gpl")) {
                if (videoArea.externalSdkModal === null) {
                    const licenseUrl = "https://code.videolan.org/videolan/x264/-/raw/master/COPYING";
                    // const licenseUrl = "https://bitbucket.org/multicoreware/x265_git/raw/master/COPYING";
                    const dlg = messageBox(Modal.Info, qsTr("This encoder requires an external library licensed as GPL.\nDo you agree with the [GPL license] and want to download the additional codec?").replace(/\[(.*?)\]/, '<a href="' + licenseUrl + '"><font color="' + styleTextColor + '">$1</font></a>'), [
                        { text: qsTr("Yes, I agree"), accent: true, clicked: function() {
                            dlg.btnsRow.children[0].enabled = false;
                            controller.install_external_sdk("ffmpeg_gpl");
                            return false;
                        } },
                        { text: qsTr("Cancel"), clicked: function() {
                            videoArea.externalSdkModal = null;
                        } },
                    ]);
                    videoArea.externalSdkModal = dlg;
                    dlg.addLoader();
                }
                return "";
            }

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
        if (text.startsWith("file_exists:")) {
            return qsTr("Output file already exists, do you want to overwrite it?");
        }
        if (text.startsWith("uses_cpu")) {
            return qsTr("GPU encoder failed to initialize and rendering is done on the CPU, which is much slower.\nIf you have a modern device, latest GPU drivers and you think this shouldn't happen, report this on GitHub including gyroflow.log file.");
        }
        if (text.includes("hevc") && text.includes("-12912")) {
            return qsTr("Your GPU doesn't support H.265/HEVC encoding, try to use H.264/AVC or disable GPU encoding in Export settings.");
        }
        if (text.includes("failed to decode picture") && text.includes("-12909")) {
            return qsTr("GPU decoder failed to decode this file. Disable GPU decoding in \"Advanced\" and try again.") + "\n\n" + text;
        }
        if (text.includes("codec not currently supported in container")) {
            return qsTr("Make sure your output extension supports the selected codec. \".mov\" should work in most cases.") + "\n\n" + text;
        }
        if (text.includes("[aac]") && text.includes("Invalid data found when processing input")) {
            return qsTr("Audio encoder couldn't process the input data. Try unchecking \"Export audio\" in Export settings.") + "\n\n" + text;
        }

        return text.trim();
    }

    function renameOutput(filename: string, folderUrl: url) {
        let newName = filename;
        for (let i = 1; i < 1000; ++i) {
            newName = filename.replace(/(_\d+)?((?:_%05d)?\.[a-z0-9]+)$/i, "_" + i + "$2");

            if (!filesystem.exists_in_folder(folderUrl, newName.replace("_%05d", "_00001")) && !render_queue.file_exists_in_folder(folderUrl, newName))
                break;
        }

        return newName;
    }

    function reportProgress(progress: real, type: string) {
        if (videoArea.videoLoader.active) {
            if (type === "loader") ui_tools.set_progress(progress);
            return;
        }
        ui_tools.set_progress(progress);
    }

    function getAdditionalProjectData() {
        return {
            "output": exportSettings.item.getExportOptions(),
            "synchronization": sync.item.getSettings(),

            "muted": window.videoArea.vid.muted,
            "playback_speed": window.videoArea.vid.playbackRate
        };
    }
    function getAdditionalProjectDataJson(): string { return JSON.stringify(getAdditionalProjectData()); }

    function saveProjectToUrl(url: url, type: string) {
        videoArea.videoLoader.show(qsTr("Saving..."), false);
        controller.export_gyroflow_file(url, type, window.getAdditionalProjectData());
    }
    function saveProject(type: string) {
        if (!type) type = "WithGyroData";

        if (controller.project_file_url) // Always overwrite
            return saveProjectToUrl(controller.project_file_url, type);

        const folder = filesystem.get_folder(controller.input_file_url);
        const filename = filesystem.filename_with_extension(filesystem.get_filename(controller.input_file_url), "gyroflow");

        if (!filesystem.exists_in_folder(folder, filename)) {
            getSaveFileUrl(folder, filename, function(url) { saveProjectToUrl(url, type); });
        } else {
            messageBox(Modal.Question, qsTr("`.gyroflow` file already exists, what do you want to do?"), [
                { text: qsTr("Overwrite"), "accent": true, clicked: () => {
                    getSaveFileUrl(folder, filename, function(url) { saveProjectToUrl(url, type); });
                } },
                { text: qsTr("Rename"), clicked: () => {
                    let newGfFilename = filename;
                    let i = 1;
                    while (filesystem.exists_in_folder(folder, newGfFilename)) {
                        newGfFilename = filename.replace(/(_\d+)?\.([a-z0-9]+)$/i, "_" + i++ + ".$2");
                        if (i > 1000) break;
                    }

                    const suffix = advanced.item.defaultSuffix.text;
                    const newFilename = outputFile.filename.replace(new RegExp(suffix + "(_\\d+)?\\.([a-z0-9]+)$", "i"), suffix + "_" + (i - 1) + ".$2");
                    if (!filesystem.exists_in_folder(folder, newFilename)) {
                        outputFile.setFilename(newFilename);
                    }
                    getSaveFileUrl(folder, newGfFilename, function(url) { saveProjectToUrl(url, type); });
                } },
                { text: qsTr("Choose a different location"), clicked: () => {
                    gfFileDialog.projectType = type;
                    gfFileDialog.currentFolder = folder;
                    gfFileDialog.open();
                } },
                { text: qsTr("Cancel") }
            ], undefined, Text.MarkdownText);
        }
    }
    function getSaveFileUrl(folder: url, filename: string, cb) {
        if (isSandboxed) {
            const opf = Qt.createComponent("components/OutputPathField.qml").createObject(window, { visible: false });
            opf.selectFolder(folder, function(folder_url) {
                cb(filesystem.get_file_url(folder_url, filename, true));
                opf.destroy();
            });
            return;
        }
        cb(filesystem.get_file_url(folder, filename, true));
    }

    /*Row {
        id: fps;
        property int frameCounter: 0;
        property int fps: 0;
        Image {
            id: spinnerImage;
            width: 2; height: 2;
            source: "qrc:/resources/logo_black.svg";
            NumberAnimation on rotation { from: 0; to: 360; duration: 800; loops: Animation.Infinite }
            onRotationChanged: fps.frameCounter++;
        }
        Text { color: "red"; font.pixelSize: 18; text: fps.fps + " fps"; }
        Timer {
            interval: 2000;
            repeat: true;
            running: true;
            onTriggered: {
                fps.fps = fps.frameCounter / 2;
                fps.frameCounter = 0;
            }
        }
    }*/
}
