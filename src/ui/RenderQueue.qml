// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC

import "components/"
import "Util.js" as Util;

Item {
    id: root;

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
        opacity: 0.7;
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
        icon.name: "close";
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
            height: parent.height;
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
                height: 20 * dpiScale;
                id: totalTime;
                property string elapsed: "---";
                property string remaining: "---";
                property real fps: 0;
                property string fpsText: topCol.progress > 0? qsTr(" @ %1fps").arg(fps.toFixed(1)) : "";
                BasicText {
                    leftPadding: 0;
                    text: qsTr("Elapsed: %1").arg("<b>" + totalTime.elapsed + "</b>");
                }
                BasicText {
                    leftPadding: 0;
                    anchors.horizontalCenter: parent.horizontalCenter;
                    textFormat: Text.RichText;
                    text: `<b>${(topCol.progress*100).toFixed(2)}%</b> <small>(${render_queue.current_frame}/${render_queue.total_frames}${totalTime.fpsText})</small>`;
                }
                BasicText {
                    leftPadding: 0;
                    anchors.right: parent.right;
                    text: qsTr("Remaining: %1").arg("<b>" + (render_queue.status == "active"? totalTime.remaining : "---") + "</b>");
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
                    messageBox(Modal.Error, text, [ { "text": qsTr("Ok"), clicked: window[callback] } ]);
                }
                delete loader.pendingJobs[job_id];
                loader.updateStatus();
            }
            function onRender_progress(job_id: real, progress: real, frame: int, total_frames: int, finished: bool) {
                if (job_id == render_queue.main_job_id) {
                    window.videoArea.videoLoader.active = !finished;
                    window.videoArea.videoLoader.currentFrame = frame;
                    window.videoArea.videoLoader.totalFrames = total_frames;
                    window.videoArea.videoLoader.additional = "";
                    window.videoArea.videoLoader.text = window.videoArea.videoLoader.active? qsTr("Rendering %1...") : "";
                    window.videoArea.videoLoader.progress = window.videoArea.videoLoader.active? progress : -1;
                    window.videoArea.videoLoader.cancelable = true;

                    if (total_frames > 0 && finished) {
                        render_queue.main_job_id = 0;
                        const path = render_queue.get_job_output_path(job_id);
                        messageBox(Modal.Success, qsTr("Rendering completed. The file was written to: %1.").arg("<br><b>" + path + "</b>"), [
                            { text: qsTr("Open rendered file"), clicked: () => controller.open_file_externally(path) },
                            { text: qsTr("Open file location"), clicked: () => controller.open_file_externally(Util.getFolder(path)) },
                            { text: qsTr("Ok") }
                        ]);
                    }
                }
            }
            function onConvert_format(job_id: real, format: string, supported: string) {
                if (job_id == render_queue.main_job_id) {
                    let buttons = supported.split(",").map(f => ({
                        text: f,
                        clicked: () => {
                            render_queue.set_pixel_format(job_id, f);
                            render_queue.render_job(job_id, false);
                        }
                    }));
                    buttons.push({
                        text: qsTr("Render using CPU"),
                        accent: true,
                        clicked: () => {
                            render_queue.set_pixel_format(job_id, "cpu");
                            render_queue.render_job(job_id, false);
                        }
                    });
                    buttons.push({ text: qsTr("Cancel") });

                    messageBox(Modal.Question, qsTr("GPU accelerated encoder doesn't support this pixel format (%1).\nDo you want to convert to a different supported pixel format or keep the original one and render on the CPU?").arg(format), buttons);
                }
                delete loader.pendingJobs[job_id];
                loader.updateStatus();
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
            icon.name: statuses[status][1];
            accentColor: statuses[status][2];
            icon.width: 15 * dpiScale;
            icon.height: 15 * dpiScale;
            height: 28 * dpiScale;
            leftPadding: 8 * dpiScale;
            rightPadding: 8 * dpiScale;
            topPadding: 3 * dpiScale;
            bottomPadding: 3 * dpiScale;
            font.pixelSize: 12 * dpiScale;
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
        width: parent.width - 2*x;
        clip: true;
        model: render_queue.queue;
        spacing: 5 * dpiScale;
        QQC.ScrollIndicator.vertical: QQC.ScrollIndicator { }
        delegate: Item {
            // https://doc.qt.io/qt-6/qtquick-tutorials-dynamicview-dynamicview3-example.html
            implicitHeight: innerItm.height + 2*innerItm.y + messageAreaParent.height;
            width: parent? parent.width : 0;
            id: dlg;
            property real progress: current_frame / total_frames;
            property bool isFinished: current_frame >= total_frames && total_frames > 0;
            property bool isError: error_string.length > 0 && !isQuestion;
            property bool isQuestion: error_string.startsWith("convert_format:");
            onProgressChanged: {
                const times = Util.calculateTimesAndFps(progress, current_frame, start_timestamp);
                if (times !== false) {
                    time.elapsed = times[0];
                    time.remaining = times[1];
                    if (times.length > 2) time.fps = times[2];
                } else {
                    time.elapsed = "";
                }
            }

            MouseArea {
                anchors.fill: parent;
                acceptedButtons: Qt.LeftButton | Qt.RightButton;
                onPressAndHold: (mouse) => {
                    if ((Qt.platform.os == "android" || Qt.platform.os == "ios") && mouse.button !== Qt.RightButton) {
                        contextMenu.popup()
                    }
                }
                onClicked: (mouse) => { if (mouse.button === Qt.RightButton) contextMenu.popup(); }
            }
            Menu {
                id: contextMenu;
                font.pixelSize: 11.5 * dpiScale;
                Action {
                    icon.name: "play";
                    text: qsTr("Render now");
                    enabled: !isFinished;
                    onTriggered: render_queue.render_job(job_id, true);
                }
                Action {
                    icon.name: "pencil";
                    text: qsTr("Edit");
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
                    icon.name: "spinner";
                    text: qsTr("Reset status");
                    enabled: isError || isFinished || isQuestion;
                    onTriggered: render_queue.reset_job(job_id);
                }
            }

            Rectangle {
                anchors.fill: parent;
                color: styleBackground2
                opacity: 0.2;
                radius: 5 * dpiScale;
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
                            text: window.getReadableError(error_string).replace(/\n/g, "<br>");
                        }
                        Flow {
                            id: messageBtns;
                            visible: btns.model.length > 0;
                            spacing: 5 * dpiScale;
                            width: parent.width;
                            property string errorString: error_string;
                            onErrorStringChanged: {
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
                    active: (isError || isQuestion) && !isFinished;
                    sourceComponent: messageAreaComponent;
                    width: parent.width;
                }
                clip: true;
            }
            Item {
                id: innerItm;
                x: 5 * dpiScale;
                width: parent.width - 2*x;
                height: 70 * dpiScale;
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
                    x: 55 * dpiScale;
                    anchors.verticalCenter: parent.verticalCenter;
                    spacing: 3 * dpiScale;
                    BasicText {
                        text: Util.getFilename(input_file);
                        font.bold: true;
                        font.pixelSize: 14 * dpiScale;
                    }
                    BasicText { text: qsTr("Save to: %1").arg("<b>" + output_path + "</b>"); }
                    BasicText { text: qsTr("Export settings: %1").arg("<b>" + export_settings + "</b>"); }
                }

                Column {
                    anchors.right: btnsRow.left;
                    anchors.rightMargin: 10 * dpiScale;
                    spacing: 6 * dpiScale;
                    anchors.verticalCenter: parent.verticalCenter;

                    BasicText {
                        leftPadding: 0;
                        anchors.horizontalCenter: parent.horizontalCenter;
                        horizontalAlignment: Text.AlignHCenter;
                        textFormat: Text.RichText;
                        text: `<b>${(dlg.progress*100).toFixed(2)}%</b> <small>(${current_frame}/${total_frames}${time.fpsText})</small>`;
                    }
                    QQC.ProgressBar {
                        width: 200 * dpiScale;
                        value: current_frame / total_frames;
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
                        text: qsTr("Elapsed: %1. Remaining: %2").arg("<b>" + elapsed + "</b>").arg("<b>" + (statusBg.shown? "---" : remaining) + "</b>");
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
                            visible: dlg.isFinished;
                            icon.name: "play";
                            icon.width: 25 * dpiScale;
                            icon.height: 25 * dpiScale;
                            tooltip: qsTr("Open rendered file");
                            onClicked: controller.open_file_externally(output_path);
                        }
                        IconButton {
                            visible: dlg.isFinished;
                            icon.name: "folder";
                            tooltip: qsTr("Open file location");
                            onClicked: controller.open_file_externally(Util.getFolder(output_path));
                        }
                        IconButton {
                            tooltip: qsTr("Remove");
                            textColor: "#f67575"
                            icon.name: dlg.isFinished? "close" : "bin";
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
        color: styleBackground2;
        anchors.margins: 0 * dpiScale;
        anchors.topMargin: lv.y;
        extensions: fileDialog.extensions;
        onLoadFiles: (urls) => {
            const options = JSON.stringify(exportSettings.getExportOptions());

            for (const url of urls) {
                const job_id = render_queue.add_file(url, controller, options);
                loader.pendingJobs[job_id] = true;
            }
            loader.updateStatus();
        }
    }

    LoaderOverlay {
        id: loader;
        active: false;
        property var pendingJobs: ({});
        function updateStatus() { active = Object.keys(pendingJobs).length > 0; }
    }
}
