// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick

Column {
    spacing: parent.spacing;
    width: parent.width;

    property real diff: -10 * dpiScale;
    default property alias data: advanced.data;

    LinkButton {
        text: qsTr("Advanced");
        anchors.horizontalCenter: parent.horizontalCenter;
        onClicked: advanced.opened = !advanced.opened;
    }
    Column {
        spacing: parent.spacing;
        id: advanced;
        property bool opened: false;
        width: parent.width;
        visible: opacity > 0;
        opacity: opened? 1 : 0;
        height: opened? implicitHeight : diff
        Ease on opacity { }
        Ease on height { id: anim; }
        onOpenedChanged: {
            anim.enabled = true;
            timer.start();
        }
        Timer {
            id: timer;
            interval: 700;
            onTriggered: anim.enabled = false;
        }
    }
}
