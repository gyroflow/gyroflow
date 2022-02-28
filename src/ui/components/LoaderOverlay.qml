// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC

Item {
    id: root;
    property bool active: false;
    property real progress: -1;
    opacity: active? 1 : 0;
    Ease on opacity { duration: 1000; }
    property string text;
    property alias t: t;
    property bool cancelable: true;
    property bool canceled: false;
    property real startTime: 0;
    property int currentFrame: 0;
    property int totalFrames: 0;
    property string additional;

    //onActiveChanged: parent.opacity = Qt.binding(() => (1.5 - opacity));
    onActiveChanged: {
        time.elapsed = "";
        time.remaining = "";
        if (!active) {
            progress = -1;
            root.text = "";
            startTime = 0;
        } else {
            canceled = false;
            startTime = Date.now();
        }
    }
    function timeToStr(v) {
        const d = Math.floor((v %= 31536000) / 86400),
              h = Math.floor((v %= 86400) / 3600),
              m = Math.floor((v %= 3600) / 60),
              s = Math.round(v % 60);

        if (d || h || m || s) {
            return (d? d + qsTr("d") + " " : "") +
                   (h? h + qsTr("h") + " " : "") +
                   (m? m + qsTr("m") + " " : "") +
                    s + qsTr("s");
        }
        return qsTr("&lt; 1s");
    }
    onProgressChanged: {
        if (progress > 0 && progress <= 1.0 && startTime > 0) {
            const elapsedMs = Date.now() - startTime;
            const totalEstimatedMs = elapsedMs / progress;
            const remainingMs = totalEstimatedMs - elapsedMs;
            if (remainingMs > 5 || elapsedMs > 5) {
                time.elapsed = timeToStr(elapsedMs / 1000);
                time.remaining = timeToStr(remainingMs / 1000);
            }
            ui_tools.set_progress(progress);

            if (elapsedMs > 5 && root.currentFrame > 0) {
                time.fps = root.currentFrame / (elapsedMs / 1000.0);
            }
        } else {
            ui_tools.set_progress(-1.0);
            time.elapsed = "";
        }
    }

    signal cancel();

    Rectangle {
        anchors.fill: parent;
        color: styleBackground2;
        opacity: 0.8;
    }

    anchors.fill: parent;
    QQC.ProgressBar { id: pb; anchors.centerIn: parent; value: parent.progress; visible: parent.progress != -1 && !root.canceled; }
    QQC.BusyIndicator { id: bi; anchors.centerIn: parent; visible: parent.active && (parent.progress == -1 || root.canceled); }

    Column {
        id: c;
        anchors.top: pb.visible? pb.bottom : bi.bottom;
        anchors.topMargin: 8 * dpiScale;
        width: parent.width;
        BasicText {
            id: t;
            text: root.text? root.text.arg("<b>" + (Math.min(root.progress, 1.0) * 100).toFixed(2) + "%</b>") + ` <font size="2">(${root.currentFrame}/${root.totalFrames}${root.additional}${time.fpsText})</font>` : "";
            visible: text.length > 0;
            width: parent.width;
            font.pixelSize: 14 * dpiScale;
            horizontalAlignment: Text.AlignHCenter;
            topPadding: 8 * dpiScale;
            bottomPadding: 5 * dpiScale;
        }
        BasicText {
            id: time;
            property string elapsed: "";
            property string remaining: "";
            property real fps: 0;
            property string fpsText: root.progress > 0? qsTr(" @ %1fps").arg(fps.toFixed(1)) : "";
            text: qsTr("Elapsed: %1. Remaining: %2").arg("<b>" + elapsed + "</b>").arg("<b>" + remaining + "</b>");
            visible: elapsed.length > 0 && remaining.length > 0;
            font.pixelSize: 11 * dpiScale;
            anchors.horizontalCenter: parent.horizontalCenter;
            horizontalAlignment: Text.AlignHCenter;
            topPadding: 0;
            bottomPadding: 4 * dpiScale;
        }
        LinkButton {
            transparent: true;
            visible: progress > -1 && cancelable;
            text: qsTr("Cancel");
            anchors.horizontalCenter: parent.horizontalCenter;
            onClicked: { root.canceled = true; root.cancel(); }
        }
    }
}
