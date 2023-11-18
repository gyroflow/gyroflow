// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC
import Qt.labs.settings

import "components/"
import "Util.js" as Util;

Item {
    id: root;

    property alias dt: dt;
    property bool shown: false;
    opacity: shown? 1 : 0;
    visible: opacity > 0;
    anchors.bottomMargin: (shown? 10 : 30) * dpiScale;
    anchors.topMargin: (shown? 10 : -20) * dpiScale;
    Ease on opacity { }
    Ease on anchors.bottomMargin { }
    Ease on anchors.topMargin { }

    MouseArea {
        anchors.fill: parent;
        preventStealing: true;
    }

    Rectangle {
        color: styleBackground2
        opacity: 0.85;
        anchors.fill: parent;
        radius: 5 * dpiScale;
        border.width: 1;
        border.color: styleVideoBorderColor;
    }

    BasicText {
        y: 12 * dpiScale;
        x: 5 * dpiScale;
        text: qsTr("Render queue");
        font.pixelSize: 15 * dpiScale;
        font.bold: true;
    }

    LinkButton {
        anchors.right: parent.right;
        width: 34 * dpiScale;
        height: 34 * dpiScale;
        textColor: styleTextColor;
        iconName: "close";
        leftPadding: 0;
        rightPadding: 0;
        topPadding: 10 * dpiScale;
        onClicked: root.shown = false;
    }

    Hr { width: parent.width - 10 * dpiScale; y: 35 * dpiScale; color: "#fff"; opacity: 0.3; }

    Row {
        id: progressRow;
        y: 55 * dpiScale;
        spacing: 10 * dpiScale;
        x: 10 * dpiScale;
        Column {
            id: topCol;
            spacing: 5 * dpiScale;
            width: parent.parent.width - x - mainBtn.width - 3 * parent.spacing;
            property real progress: Math.max(0, Math.min(1, render_queue.current_frame / Math.max(1, render_queue.total_frames)));
            onProgressChanged: {
                const times = Util.calculateTimesAndFps(progress, render_queue.current_frame, render_queue.start_timestamp, render_queue.end_timestamp);
                if (times !== false && progress < 1.0) {
                    totalTime.elapsed = times[0];
                    totalTime.remaining = times[1];
                    if (times.length > 2) totalTime.fps = times[2];
                    window.reportProgress(progress, "queue");
                } else {
                    window.reportProgress(-1, "queue");
                    totalTime.remaining = "---";
                }
            }

            Item {
                width: parent.width;
                height: (twoLines? 35 : 20) * dpiScale;
                id: totalTime;
                property string elapsed: "---";
                property string remaining: "---";
                property real fps: 0;
                property string fpsText: topCol.progress > 0? qsTr(" @ %1fps").arg(fps.toFixed(1)) : "";
                onWidthChanged: Qt.callLater(totalTime.updateLayout);
                property bool twoLines: false;
                function updateLayout() {
                    const totalTextSize = progressText1.width + progressText2.width + progressText3.width + 25 * dpiScale;
                    twoLines = totalTextSize > totalTime.width;
                }

                BasicText {
                    id: progressText1;
                    leftPadding: 0;
                    text: qsTr("Elapsed: %1").arg("<b>" + totalTime.elapsed + "</b>");
                    onWidthChanged: Qt.callLater(totalTime.updateLayout);
                }
                BasicText {
                    id: progressText2;
                    leftPadding: 0;
                    anchors.horizontalCenter: parent.horizontalCenter;
                    textFormat: Text.RichText;
                    text: `<b>${(topCol.progress*100).toFixed(2)}%</b> <small>(${render_queue.current_frame}/${render_queue.total_frames}${totalTime.fpsText})</small>`;
                    y: totalTime.twoLines? progressText1.height + 5 * dpiScale : 0;
                    onWidthChanged: Qt.callLater(totalTime.updateLayout);
                }
                BasicText {
                    id: progressText3;
                    leftPadding: 0;
                    anchors.right: parent.right;
                    text: qsTr("Remaining: %1").arg("<b>" + (render_queue.status == "active"? totalTime.remaining : "---") + "</b>");
                    onWidthChanged: Qt.callLater(totalTime.updateLayout);
                }
            }
            QQC.ProgressBar {
                id: pb;
                width: parent.width;
                value: topCol.progress;
            }
        }
        Connections {
            target: render_queue;
            function onAdded(job_id: real) {
                delete loader.pendingJobs[job_id];
                loader.updateStatus();
            }
            function onError(job_id: real, text: string, arg: string, callback: string) {
                if (job_id == render_queue.main_job_id || loader.pendingJobs[job_id]) {
                    text = getReadableError(qsTr(text).arg(arg));
                    if (text) {
                        // if (text.includes("failed to decode picture"))
                        //     window.advanced.gpudecode.checked = false;
                        messageBox(Modal.Error, text, [ { "text": qsTr("Ok"), clicked: window[callback] } ]);
                    }
                }
                delete loader.pendingJobs[job_id];
                loader.updateStatus();
            }
            function onRender_progress(job_id: real, progress: real, frame: int, total_frames: int, finished: bool, start_time: real, is_conversion: bool) {
                if (job_id == render_queue.main_job_id) {
                    window.videoArea.videoLoader.active = !finished;
                    window.videoArea.videoLoader.currentFrame = frame;
                    window.videoArea.videoLoader.totalFrames = total_frames;
                    window.videoArea.videoLoader.additional = "";
                    window.videoArea.videoLoader.text = window.videoArea.videoLoader.active? (is_conversion? qsTr("Converting to %1 %2...").arg(window.advanced.r3dConvertFormat.currentText) : qsTr("Rendering %1...")) : "";
                    window.videoArea.videoLoader.progress = window.videoArea.videoLoader.active? progress : -1;
                    window.videoArea.videoLoader.cancelable = true;
                    window.videoArea.videoLoader.startTime = start_time;

                    if (total_frames > 0 && finished) {
                        render_queue.main_job_id = 0;
                        const folder = render_queue.get_job_output_folder(job_id);
                        const filename = render_queue.get_job_output_filename(job_id);
                        let options = [];
                        if (Qt.platform.os != "ios") {
                            options.push({ text: qsTr("Open rendered file"), clicked: () => filesystem.open_file_externally(filesystem.get_file_url(folder, filename, false)) });
                        }
                        if (Qt.platform.os != "android" && Qt.platform.os != "ios") {
                            options.push({ text: qsTr("Open file location"), clicked: () => filesystem.open_file_externally(folder) });
                        }
                        options.push({ text: qsTr("Ok") });

                        messageBox(Modal.Success, qsTr("Rendering completed. The file was written to: %1.").arg("<br><b>" + filesystem.display_folder_filename(folder, filename) + "</b>"), options);
                    }
                }
            }
            function onConvert_format(job_id: real, format: string, supported: string) {
                if (job_id == render_queue.main_job_id) {
                    let buttons = supported.split(",").map(f => ({
                        text: f,
                        clicked: () => {
                            render_queue.set_pixel_format(job_id, f);
                            render_queue.render_job(job_id);
                        }
                    }));
                    buttons.push({
                        text: qsTr("Render using CPU"),
                        accent: true,
                        clicked: () => {
                            render_queue.set_pixel_format(job_id, "cpu");
                            render_queue.render_job(job_id);
                        }
                    });
                    buttons.push({ text: qsTr("Cancel") });

                    messageBox(Modal.Question, qsTr("GPU accelerated encoder doesn't support this pixel format (%1).\nDo you want to convert to a different supported pixel format or keep the original one and render on the CPU?").arg(format), buttons);
                }
                delete loader.pendingJobs[job_id];
                loader.updateStatus();
            }
            function onEncoder_initialized(job_id: real, encoder_name: string) {

            }
            function onRequest_close() {
                main_window.closeConfirmed = true;
                Qt.callLater(Qt.quit);
            }
        }

        Button {
            id: mainBtn;
            accent: true;
            property string status: render_queue.status;
            property var statuses: ({
                "stopped": [qsTr("Start exporting"), "play",  styleAccentColor, "start"],
                "paused":  [qsTr("Resume"),          "play",  "#70e574",        "start"],
                "active":  [qsTr("Pause"),           "pause", "#f6a00b",        "pause"],
            })
            text: statuses[status][0];
            iconName: statuses[status][1];
            accentColor: statuses[status][2];
            icon.width: 15 * dpiScale;
            icon.height: 15 * dpiScale;
            height: 28 * dpiScale;
            leftPadding: 8 * dpiScale;
            rightPadding: 8 * dpiScale;
            topPadding: 3 * dpiScale;
            bottomPadding: 3 * dpiScale;
            font.pixelSize: 12 * dpiScale;
            anchors.verticalCenter: parent.verticalCenter;
            Component.onCompleted: contentItem.children[1].elide = Text.ElideNone;
            clip: true;
            Ease on implicitWidth { }
            Behavior on accentColor { ColorAnimation { duration: 700; easing.type: Easing.OutExpo; } }
            onClicked: render_queue[statuses[status][3]]();
        }
    }

    ListView {
        id: lv;
        x: 10 * dpiScale;
        anchors.top: progressRow.bottom;
        anchors.bottom: parent.bottom;
        anchors.margins: 15 * dpiScale;
        anchors.bottomMargin: 30 * dpiScale;
        width: parent.width - 2*x;
        clip: true;
        model: render_queue.queue;
        Timer {
            interval: 100;
            running: !isCalibrator && window.exportSettings != null && window.sync != null;
            onTriggered: {
                const saved = window.settings.value("renderQueue");
                if (saved && saved.length > 10) {
                    Qt.callLater(() => {
                        render_queue.restore_render_queue(saved, window.getAdditionalProjectDataJson());
                        messageBox(Modal.Info, qsTr("You have unfinished tasks in the render queue."), [
                            { text: qsTr("Open render queue"), accent: true, clicked: function() {
                                videoArea.queue.shown = true;
                            } },
                            { text: qsTr("Ok") }
                        ]);
                    });
                }
            }
        }

        Connections {
            target: render_queue;
            function onQueue_changed() {
                window.settings.setValue("renderQueue", render_queue.render_queue_json());
            }
            function onStatus_changed() {
                window.settings.setValue("renderQueue", render_queue.render_queue_json());
            }
        }
        spacing: 5 * dpiScale;
        QQC.ScrollIndicator.vertical: QQC.ScrollIndicator { }
        delegate: Item {
            // https://doc.qt.io/qt-6/qtquick-tutorials-dynamicview-dynamicview3-example.html
            implicitHeight: innerItm.height + 2*innerItm.y + messageAreaParent.height;
            width: parent? parent.width : 0;
            id: dlg;
            property real progress: current_frame / total_frames;
            property bool isFinished: current_frame >= total_frames && total_frames > 0;
            property bool isError: error_string.length > 0 && !isQuestion && !isInfo;
            property bool isInfo: error_string == "uses_cpu";
            property bool isQuestion: error_string.startsWith("convert_format:") || error_string.startsWith("file_exists:");
            property bool isInProgress: (!isFinished && !isError && !isQuestion && total_frames > 0) && (current_frame > 0 || isProcessing);
            property bool isProcessing: processing_progress > 0.0 && processing_progress < 1.0;
            property string errorString: error_string;
            property real basicTextSize: (window.isMobileLayout? 10 : 12) * dpiScale;
            onProgressChanged: {
                const times = Util.calculateTimesAndFps(progress, current_frame, start_timestamp);
                if (times !== false) {
                    time.elapsed = times[0];
                    time.remaining = times[1];
                    if (times.length > 2) time.fps = times[2];
                    if (start_timestamp_frame > 0 && start_timestamp2 > 0) {
                        const progress2 = (current_frame - start_timestamp_frame) / (total_frames - start_timestamp_frame);
                        const avgTimes = Util.calculateTimesAndFps(progress2, current_frame - start_timestamp_frame, start_timestamp2);
                        if (avgTimes !== false) {
                            time.remaining = avgTimes[1];
                            if (avgTimes.length > 2) time.fps = avgTimes[2];
                        }
                    }
                } else {
                    time.elapsed = "";
                }
            }
            onErrorStringChanged: {
                if (job_id == render_queue.main_job_id && error_string == "uses_cpu") {
                    window.videoArea.videoLoader.infoMessage.type = InfoMessage.Warning;
                    window.videoArea.videoLoader.infoMessage.text = window.getReadableError(error_string);
                    window.videoArea.videoLoader.infoMessage.show = true;
                }
            }

            ContextMenuMouseArea {
                onContextMenu: (isHold, x, y) => contextMenu.popup(dlg, x, y)
            }
            Menu {
                id: contextMenu;
                font.pixelSize: 11.5 * dpiScale;
                Action {
                    iconName: "play";
                    text: qsTr("Render now");
                    enabled: !isFinished && !isInProgress;
                    onTriggered: render_queue.render_job(job_id);
                }
                Action {
                    iconName: "pencil";
                    text: qsTr("Edit");
                    enabled: !isInProgress;
                    onTriggered:{
                        const data = render_queue.get_gyroflow_data(job_id);
                        if (data) {
                            window.videoArea.loadGyroflowData(JSON.parse(data));
                        }
                        render_queue.editing_job_id = job_id;
                        root.shown = false;
                    }
                }
                Action {
                    iconName: isInProgress? "close" : "spinner";
                    text: isInProgress? qsTr("Stop") : qsTr("Reset status");
                    enabled: isError || isFinished || isQuestion || isInProgress;
                    onTriggered: render_queue.reset_job(job_id);
                }
            }

            Rectangle {
                anchors.fill: parent;
                color: styleBackground2;
                opacity: 0.2;
                radius: 5 * dpiScale;
                border.width: window.isMobileLayout && !statusBg.shown? 1 * dpiScale : 0;
                border.color: "#70ffffff";
            }
            Item {
                height: parent.height;
                width: ipb.value * parent.width;
                clip: true;
                visible: opacity > 0;
                opacity: window.isMobileLayout && !statusBg.shown? 1 : 0;
                Ease on opacity { }
                Rectangle {
                    width: parent.parent.width;
                    height: parent.height;
                    radius: 5 * dpiScale;
                    color: "#70e574";
                    opacity: 0.35;
                }
            }
            Rectangle {
                id: statusBg;
                anchors.fill: parent;
                color: "#30" + border.color.toString().substring(1);
                radius: 5 * dpiScale;
                opacity: shown? 0.8 : 0;
                Ease on opacity { }
                property bool shown: isFinished || isError || isQuestion;
                visible: opacity > 0;
                border.color: isFinished? "#70e574" : isError? "#ed7676" : isQuestion? styleAccentColor : "transparent";
                border.width: 1;
            }

            Component {
                id: messageAreaComponent;
                Item {
                    height: messageAreaCol.height + 20 * dpiScale;
                    Hr { y: 2; color: statusBg.border.color; opacity: 0.2; }

                    Column {
                        id: messageAreaCol;
                        width: parent.width;
                        spacing: 10 * dpiScale;
                        y: 10 * dpiScale;

                        BasicText {
                            id: messageAreaText;
                            textFormat: Text.RichText;
                            leftPadding: 0;
                            font.pixelSize: basicTextSize;
                        }
                        Flow {
                            id: messageBtns;
                            visible: btns.model.length > 0;
                            spacing: 5 * dpiScale;
                            width: parent.width;
                            property string errorString: error_string;
                            onErrorStringChanged: {
                                const text = window.getReadableError(errorString).replace(/\n/g, "<br>");
                                messageAreaText.text = text? text : qsTr("Missing required components.");

                                if (errorString.startsWith("convert_format:")) {
                                    const params = errorString.split(":")[1].split(";");
                                    const supported = params[1].split(",");
                                    let buttons = supported.map(f => ({
                                        text: f,
                                        clicked: () => { render_queue.set_pixel_format(job_id, f); }
                                    }));
                                    buttons.push({
                                        text: qsTr("Render using CPU"),
                                        accent: true,
                                        clicked: () => { render_queue.set_pixel_format(job_id, "cpu"); }
                                    });
                                    btns.model = buttons;
                                } else if (errorString.startsWith("file_exists:")) {
                                    const data = JSON.parse(errorString.substring(12));
                                    switch (render_queue.overwrite_mode) {
                                        case 1: render_queue.reset_job(job_id); btns.model = []; break; // Overwrite
                                        case 2: render_queue.set_job_output_filename(job_id, window.renameOutput(data.filename, data.folder), false); btns.model = []; break; // Rename
                                        case 3: render_queue.set_error_string(job_id, qsTr("Output file already exists.")); btns.model = []; break; // Skip
                                        default:
                                            btns.model = [
                                                { text: qsTr("Yes"),    clicked: () => { render_queue.reset_job(job_id); }, accent: true },
                                                { text: qsTr("Rename"), clicked: () => { render_queue.set_job_output_filename(job_id, window.renameOutput(data.filename, data.folder), true); } },
                                                { text: qsTr("No"),     clicked: () => { render_queue.set_error_string(job_id, qsTr("Output file already exists.")); btns.model = []; } },
                                            ];
                                        break;
                                    }
                                }
                            }
                            Repeater {
                                id: btns;
                                model: []
                                Button {
                                    text: modelData.text;
                                    height: 25 * dpiScale;
                                    accent: modelData.accent || false;
                                    leftPadding: 12 * dpiScale;
                                    rightPadding: 12 * dpiScale;
                                    font.pixelSize: 12 * dpiScale;
                                    onClicked: modelData.clicked();
                                }
                            }
                        }
                    }
                }
            }
            Item {
                id: messageAreaParent;
                visible: height > 0;
                anchors.bottom: parent.bottom;
                width: parent.width - 2*x;
                x: 15 * dpiScale;
                height: messageArea.active? messageArea.height : 0;
                Ease on height { }
                Loader {
                    id: messageArea;
                    active: (isError || isQuestion || isInfo) && !isFinished;
                    sourceComponent: messageAreaComponent;
                    width: parent.width;
                }
                clip: true;
            }
            Item {
                id: innerItm;
                x: 5 * dpiScale;
                width: parent.width - 2*x;
                height: textColumn.height + 20 * dpiScale;
                Image {
                    x: 5 * dpiScale;
                    source: thumbnail_url
                    fillMode: Image.PreserveAspectCrop
                    width: 50 * dpiScale;
                    height: 50 * dpiScale;
                    anchors.verticalCenter: parent.verticalCenter;
                    Rectangle {
                        anchors.fill: parent;
                        anchors.margins: -1 * dpiScale;
                        color: "transparent";
                        radius: 5 * dpiScale;
                        anchors.verticalCenter: parent.verticalCenter;
                        border.width: 1 * dpiScale;
                        border.color: styleVideoBorderColor
                    }
                    QQC.BusyIndicator { anchors.centerIn: parent; visible: !thumbnail_url; scale: 0.5; running: visible; }
                }

                Column {
                    id: textColumn;
                    x: 55 * dpiScale;
                    anchors.verticalCenter: parent.verticalCenter;
                    spacing: 3 * dpiScale;
                    width: parent.width - x - btnsRow.width - 10 * dpiScale;
                    BasicText {
                        text: input_filename;
                        font.bold: true;
                        font.pixelSize: 14 * dpiScale;
                        width: parent.width;
                        wrapMode: Text.WordWrap;
                    }
                    BasicText {
                        visible: window.isMobileLayout;
                        width: parent.width;
                        wrapMode: Text.WordWrap;
                        font.pixelSize: basicTextSize;
                        property string remainingText: statusBg.shown? "---" : time.remaining;
                        property string eta: remainingText != "---"? (", " + qsTr("ETA %1").arg(remainingText)) : "";
                        text: isProcessing? qsTr("Synchronizing: %1").arg(`<b>${(processing_progress*100).toFixed(2)}%</b>`)
                                          : qsTr("Rendering: %1").arg(`<b>${(dlg.progress*100).toFixed(2)}%</b> <small>(${current_frame}/${total_frames}${time.fpsText}${eta})</small>`);
                    }
                    BasicText { text: qsTr("Save to: %1").arg("<b>" + display_output_path + "</b>"); font.pixelSize: basicTextSize; width: parent.width; wrapMode: Text.WordWrap; }
                    BasicText { text: qsTr("Export settings: %1").arg("<b>" + export_settings + "</b>"); font.pixelSize: basicTextSize; width: parent.width; wrapMode: Text.WordWrap; }
                }

                Column {
                    anchors.right: btnsRow.left;
                    anchors.rightMargin: 10 * dpiScale;
                    spacing: 6 * dpiScale;
                    anchors.verticalCenter: parent.verticalCenter;
                    visible: !window.isMobileLayout;

                    BasicText {
                        leftPadding: 0;
                        anchors.horizontalCenter: parent.horizontalCenter;
                        horizontalAlignment: Text.AlignHCenter;
                        textFormat: Text.RichText;
                        text: isProcessing? `<b>${(processing_progress*100).toFixed(2)}%</b>` :
                                            `<b>${(dlg.progress*100).toFixed(2)}%</b> <small>(${current_frame}/${total_frames}${time.fpsText})</small>`;
                    }
                    QQC.ProgressBar {
                        id: ipb;
                        width: 200 * dpiScale;
                        value: isProcessing? processing_progress : current_frame / total_frames;
                    }
                    BasicText {
                        id: time;
                        property string elapsed: "---";
                        property string remaining: "---";
                        property real fps: 0;
                        property string fpsText: dlg.progress > 0? qsTr(" @ %1fps").arg(fps.toFixed(1)) : "";
                        leftPadding: 0;
                        anchors.horizontalCenter: parent.horizontalCenter;
                        horizontalAlignment: Text.AlignHCenter;
                        text: isProcessing? qsTr("Synchronizing...")
                                          : qsTr("Elapsed: %1. Remaining: %2").arg("<b>" + elapsed + "</b>").arg("<b>" + (statusBg.shown? "---" : remaining) + "</b>");
                    }
                }

                Item {
                    id: btnsRow;
                    anchors.right: parent.right;
                    anchors.verticalCenter: parent.verticalCenter;
                    width: btnsRowInner.width;
                    height: btnsRowInner.height;
                    Ease on width { }

                    component IconButton: LinkButton {
                        width: 30 * dpiScale;
                        height: 30 * dpiScale;
                        textColor: styleAccentColor;
                        icon.width: 15 * dpiScale;
                        icon.height: 15 * dpiScale;
                        leftPadding: 0;
                        rightPadding: 0;
                        font.underline: false;
                        font.bold: true;
                        Ease on opacity { duration: 300; }
                        opacity: pressed? 0.8 : 1;
                    }

                    Row {
                        id: btnsRowInner;
                        IconButton {
                            visible: dlg.isFinished && Qt.platform.os != "ios";
                            iconName: "play";
                            icon.width: 25 * dpiScale;
                            icon.height: 25 * dpiScale;
                            tooltip: qsTr("Open rendered file");
                            onClicked: filesystem.open_file_externally(filesystem.get_file_url(output_folder, output_filename, false));
                        }
                        IconButton {
                            visible: dlg.isFinished && Qt.platform.os != "android" && Qt.platform.os != "ios";
                            iconName: "folder";
                            tooltip: qsTr("Open file location");
                            onClicked: filesystem.open_file_externally(output_folder);
                        }
                        IconButton {
                            tooltip: qsTr("Remove");
                            textColor: "#f67575"
                            iconName: dlg.isFinished? "close" : "bin";
                            onClicked: render_queue.remove(job_id);
                        }
                    }
                }
            }
            clip: true;
        }
        highlight: Item { }
        add: Transition {
            NumberAnimation { properties: "y"; from: (lv.count - 1.5) * (70 * dpiScale); duration: 700; easing.type: Easing.OutExpo; }
            NumberAnimation { properties: "opacity"; from: 0; to: 1; duration: 700; easing.type: Easing.OutExpo; }
        }
        remove: Transition {
            NumberAnimation { properties: "opacity"; from: 1; to: 0; duration: 700; easing.type: Easing.OutExpo; }
            NumberAnimation { properties: "implicitHeight"; from: 65 * dpiScale; to: 0; duration: 700; easing.type: Easing.OutExpo; }
        }
        displaced: Transition {
            NumberAnimation { properties: "y"; duration: 700; easing.type: Easing.OutExpo; }
        }
    }

    DropTarget {
        id: dt;
        color: styleBackground2;
        anchors.margins: 0 * dpiScale;
        anchors.topMargin: lv.y;
        extensions: fileDialog.extensions;
        function add(outFolder, urls) {
            let additional = window.getAdditionalProjectData();
            if (!outFolder) {
                delete additional.output.output_folder;
                delete additional.output.output_filename;
            } else {
                additional.output.output_folder = outFolder;
            }
            additional = JSON.stringify(additional);

            for (const url of urls) {
                const job_id = render_queue.add_file(url.toString(), "", additional);
                loader.pendingJobs[job_id] = true;
            }
            loader.updateStatus();
        }
        onLoadFiles: (urls) => {
            if (!urls.length) return;
            if (filesystem.get_filename(urls[0]).toLowerCase().endsWith(".gyroflow")) {
                add("", urls);
            } else {
                window.videoArea.askForOutputLocation(window.outputFile.folderUrl, "", true, function(outFolder, _, __) { add(outFolder, urls); });
            }
        }
    }

    LinkButton {
        visible: !isMobile;
        anchors.left: parent.left;
        anchors.bottom: parent.bottom;
        anchors.margins: 5 * dpiScale;
        leftPadding: 5 * dpiScale; rightPadding: 5 * dpiScale;
        property int currentOption: 0;
        property var options: [
            QT_TRANSLATE_NOOP("Popup", "Do nothing"),
            QT_TRANSLATE_NOOP("Popup", "Shut down the computer"),
            QT_TRANSLATE_NOOP("Popup", "Restart the computer"),
            QT_TRANSLATE_NOOP("Popup", "Sleep"),
            QT_TRANSLATE_NOOP("Popup", "Hibernate"),
            QT_TRANSLATE_NOOP("Popup", "Logout"),
            QT_TRANSLATE_NOOP("Popup", "Close Gyroflow")
        ];
        text: qsTr("When rendering is finished: %1").arg(qsTranslate("Popup", options[currentOption])).trim();
        onClicked: if (p0.visible) { p0.close(); } else { p0.open(); }
        onCurrentOptionChanged: render_queue.when_done = currentOption;
        Popup {
            id: p0;
            model: parent.options;
            currentIndex: parent.currentOption;
            width: maxItemWidth + 10 * dpiScale;
            x: parent.width - width;
            y: itemHeight;
            itemHeight: 25 * dpiScale;
            font.pixelSize: 11 * dpiScale;
            onClicked: i => parent.currentOption = i;
        }
    }
    LinkButton {
        id: queueSettings;
        anchors.right: parent.right;
        anchors.bottom: parent.bottom;
        anchors.margins: 5 * dpiScale;
        leftPadding: 5 * dpiScale; rightPadding: 5 * dpiScale;
        text: qsTr("Queue settings");
        onClicked: if (queueSettingsMenu.visible) { queueSettingsMenu.dismiss(); } else { queueSettingsMenu.popup(queueSettings, 0, height); }

        function setParallelRenders(v: int, menuItem: Menu) {
            v = Math.min(6, Math.max(v, 1));

            render_queue.parallel_renders = v;

            for (let i = 0; i < menuItem.count; ++i) {
                if (menuItem.itemAt(i) instanceof QQC.MenuItem) { menuItem.actionAt(i).checked = i == v - 1; }
            }
            window.settings.setValue("parallelRenders", v);
        }
        function setOverwriteAction(v: int, menuItem: Menu) {
            v = Math.min(3, Math.max(v, 0));

            render_queue.overwrite_mode = v;

            for (let i = 0, j = 0; i < menuItem.count; ++i) {
                if (menuItem.itemAt(i) instanceof QQC.MenuItem) { menuItem.actionAt(i).checked = j == v; j++;  }
            }
            window.settings.setValue("defaultOverwriteAction", v);

        }
        function setExportMode(v: int, menuItem: Menu) {
            v = Math.min(4, Math.max(v, 0));

            render_queue.export_project = v;

            for (let i = 0; i < menuItem.count; ++i) {
                if (menuItem.itemAt(i) instanceof QQC.MenuItem) { menuItem.actionAt(i).checked = i == v; }
            }
            window.settings.setValue("exportMode", v);
        }

        Menu {
            id: queueSettingsMenu;
            Menu {
                id: parallelRendersMenu;
                title: qsTr("Number of parallel renders");
                Action { text: "1"; onTriggered: queueSettings.setParallelRenders(1, parallelRendersMenu);  }
                Action { text: "2"; onTriggered: queueSettings.setParallelRenders(2, parallelRendersMenu);  }
                Action { text: "3"; onTriggered: queueSettings.setParallelRenders(3, parallelRendersMenu);  }
                Action { text: "4"; onTriggered: queueSettings.setParallelRenders(4, parallelRendersMenu);  }
                Action { text: "5"; onTriggered: queueSettings.setParallelRenders(5, parallelRendersMenu);  }
                Action { text: "6"; onTriggered: queueSettings.setParallelRenders(6, parallelRendersMenu);  }
                Component.onCompleted: queueSettings.setParallelRenders(+window.settings.value("parallelRenders", "1"), parallelRendersMenu);
            }
            Menu {
                id: overwriteActionMenu;
                title: qsTr("Default overwrite action");
                Action { text: qsTr("Ask");            onTriggered: queueSettings.setOverwriteAction(0, overwriteActionMenu); }
                QQC.MenuSeparator { verticalPadding: 5 * dpiScale; }
                Action { text: qsTr("Overwrite file"); onTriggered: queueSettings.setOverwriteAction(1, overwriteActionMenu); }
                Action { text: qsTr("Rename file");    onTriggered: queueSettings.setOverwriteAction(2, overwriteActionMenu); }
                Action { text: qsTr("Skip file");      onTriggered: queueSettings.setOverwriteAction(3, overwriteActionMenu); }
                Component.onCompleted: queueSettings.setOverwriteAction(+window.settings.value("defaultOverwriteAction", "0"), overwriteActionMenu);
            }
            Menu {
                id: exportModeMenu;
                title: qsTr("Export mode");
                Action { text: qsTr("Stabilized video");                               onTriggered: queueSettings.setExportMode(0, exportModeMenu); }
                Action { text: qsTr("Project file");                                   onTriggered: queueSettings.setExportMode(1, exportModeMenu); }
                Action { text: qsTr("Project file (including gyro data)");             onTriggered: queueSettings.setExportMode(2, exportModeMenu); }
                Action { text: qsTr("Project file (including processed gyro data)");   onTriggered: queueSettings.setExportMode(3, exportModeMenu); }
                Action { text: qsTr("Stabilized video + Project file with gyro data"); onTriggered: queueSettings.setExportMode(4, exportModeMenu); }
                Component.onCompleted: queueSettings.setExportMode(+window.settings.value("exportMode", "0"), exportModeMenu);
            }
            QQC.MenuSeparator { verticalPadding: 5 * dpiScale; }
            Action { checked: +settings.value("showQueueWhenAdding", "1") > 0; text: qsTr("Show queue when adding an item"); onTriggered: { checked = !checked; window.settings.setValue("showQueueWhenAdding", checked? 1 : 0); } }
            Action { text: qsTr("Clear render queue"); onTriggered: {
                messageBox(Modal.Warning, qsTr("Are you sure you want to remove all items from the render queue?"), [
                    { text: qsTr("Yes"), clicked: render_queue.clear },
                    { text: qsTr("No"), accent: true },
                ]);
            } }
        }
    }

    LoaderOverlay {
        id: loader;
        active: false;
        property var pendingJobs: ({});
        function updateStatus() { active = Object.keys(pendingJobs).length > 0; }
    }
}
