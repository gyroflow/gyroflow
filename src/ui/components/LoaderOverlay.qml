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
    property alias text: t.text;
    property alias t: t;
    property bool cancelable: true;
    property bool canceled: false;
    //onActiveChanged: parent.opacity = Qt.binding(() => (1.5 - opacity));
    onActiveChanged: {
        if (!active) {
            progress = -1;
            t.text = "";
        } else {
            canceled = false;
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
    QQC.BusyIndicator { id: bi; anchors.centerIn: parent; visible: parent.progress == -1 || root.canceled; }
    
    BasicText {
        id: t;
        anchors.top: pb.visible? pb.bottom : bi.bottom;
        anchors.topMargin: 8 * dpiScale;
        visible: text.length > 0;
        width: parent.width;
        font.pixelSize: 14 * dpiScale;
        horizontalAlignment: Text.AlignHCenter;
        topPadding: 8 * dpiScale;
        bottomPadding: 8 * dpiScale;
    }

    LinkButton {
        transparent: true;
        visible: progress > -1 && cancelable;
        text: qsTr("Cancel");
        anchors.horizontalCenter: parent.horizontalCenter;
        anchors.top: t.bottom;
        onClicked: { root.canceled = true; root.cancel(); }
    }
}
