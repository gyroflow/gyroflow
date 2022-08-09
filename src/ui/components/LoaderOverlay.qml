// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC

import "../Util.js" as Util;

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
    property alias background: overlay.color;
    property bool canHide: false;
    property alias infoMessage: infoMessage;
    property alias pb: pb;

    //onActiveChanged: parent.opacity = Qt.binding(() => (1.5 - opacity));
    onActiveChanged: {
        time.elapsed = "";
        time.remaining = "";
        if (!active) {
            progress = -1;
            root.text = "";
            startTime = 0;
            infoMessage.text = "";
            infoMessage.show = false;
        } else {
            canceled = false;
            startTime = Date.now();
        }
    }
    onProgressChanged: {
        const times = Util.calculateTimesAndFps(progress, root.currentFrame, startTime);
        if (times !== false) {
            time.elapsed = times[0];
            time.remaining = times[1];
            if (times.length > 2) time.fps = times[2];
            window.reportProgress(progress, "loader");
        } else {
            window.reportProgress(-1, "loader");
            time.elapsed = "";
        }
    }

    signal cancel();
    signal hide();

    Rectangle {
        id: overlay;
        anchors.fill: parent;
        color: styleBackground2;
        opacity: 0.8;
    }

    anchors.fill: parent;
    QQC.ProgressBar { id: pb; anchors.centerIn: parent; value: parent.progress; visible: parent.progress != -1 && !root.canceled; }
    QQC.BusyIndicator { id: bi; anchors.centerIn: parent; visible: parent.active && (parent.progress == -1 || root.canceled); running: visible; }

    Column {
        id: c;
        anchors.top: pb.visible? pb.bottom : bi.bottom;
        anchors.topMargin: 8 * dpiScale;
        width: parent.width;
        BasicText {
            id: t;
            text: root.text? root.text.arg("<b>" + (Math.min(root.progress, 1.0) * 100).toFixed(2) + "%</b>") + (root.totalFrames > 0? ` <font size="2">(${root.currentFrame}/${root.totalFrames}${root.additional}${time.fpsText})</font>` : "") : "";
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
        Row {
            anchors.horizontalCenter: parent.horizontalCenter;
            property real rlPadding: (hideBtn.visible? 5 : 15) * dpiScale;
            LinkButton {
                transparent: true;
                visible: progress > -1 && cancelable;
                text: qsTr("Cancel");
                onClicked: { root.canceled = true; root.cancel(); }
                rightPadding: parent.rlPadding;
                leftPadding: parent.rlPadding;
            }
            Text {
                text: "|";
                color: styleTextColor;
                font.pixelSize: 12 * dpiScale;
                font.family: styleFont;
                visible: hideBtn.visible;
                verticalAlignment: Text.AlignVCenter;
                height: parent.height;
            }
            LinkButton {
                id: hideBtn;
                rightPadding: parent.rlPadding
                leftPadding: parent.rlPadding;
                transparent: true;
                visible: progress > -1 && canHide;
                text: qsTr("Hide");
                onClicked: root.hide();
            }
        }
        InfoMessageSmall {
            id: infoMessage;
            anchors.horizontalCenter: parent.horizontalCenter;
            shrinkToText: true;
        }
    }
}
