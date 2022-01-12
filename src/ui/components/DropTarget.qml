// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick

Rectangle {
    id: root;
    property var extensions: [];
    anchors.fill: parent;
    color: styleBackground;
    radius: 10 * dpiScale;
    opacity: da.containsDrag? 0.8 : 0.0;
    Ease on opacity { duration: 300; }

    signal loadFile(string path); 

    BasicText {
        id: dropText;
        text: qsTr("Drop file here");
        font.pixelSize: 30 * dpiScale;
        anchors.centerIn: parent;
        leftPadding: 0;
        scale: dropText.paintedWidth > (parent.width - 50 * dpiScale)? (parent.width - 50 * dpiScale) / dropText.paintedWidth : 1.0;
    }

    DropTargetRect { }

    DropArea {
        id: da;
        anchors.fill: parent;
        onEntered: (drag) => {
            const ext = drag.urls[0].toString().split(".").pop().toLowerCase();
            drag.accepted = root.extensions.indexOf(ext) > -1;
        }
        onDropped: (drop) => root.loadFile(drop.urls[0])
    }
}
