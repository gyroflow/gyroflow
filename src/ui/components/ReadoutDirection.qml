// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC
import QtQuick.Controls.impl as QQCI

Rectangle {
    id: root;
    anchors.right: parent.right;
    anchors.top: parent.top;
    anchors.topMargin: -30 * dpiScale;
    implicitHeight: 18 * dpiScale;
    implicitWidth: 23 * dpiScale;
    border.width: 1 * dpiScale;
    border.color: "#747474";
    radius: 3 * dpiScale;
    property int direction: 0; // 0 - top to bottom, 1 - bottom to top, 2 - left to right, 3 - right to left

    color: "transparent";

    function set(direction: var): void {
        switch (direction) {
            case 0: case 1: case 2: case 3: root.direction = direction; break;
            case "BottomToTop": case 180: root.direction = 1; break;
            case "LeftToRight": case 90:  root.direction = 2; break;
            case "RightToLeft": case 270: root.direction = 3; break;
            default: root.direction = 0; break;
        }
    }
    function getInt(): int {
        return root.direction;
    }
    function get(): string {
        switch (root.direction) {
            case 1: return "BottomToTop";
            case 2: return "LeftToRight";
            case 3: return "RightToLeft";
        }
        return "TopToBottom";
    }

    QQCI.IconImage {
        name: "arrow-down";
        source: "qrc:/resources/icons/svg/arrow-down.svg";
        color: styleAccentColor;
        height: Math.round(parent.height * 0.7)
        width: height;
        layer.enabled: true;
        layer.textureSize: Qt.size(height*2, height*2);
        layer.smooth: true;
        anchors.centerIn: parent;
        rotation: [0, 180, -90, 90][root.direction];
    }

    ToolTip {
        text: qsTr("Frame readout direction: %1").arg([qsTr("Top to bottom"), qsTr("Bottom to top"), qsTr("Left to right"), qsTr("Right to left")][root.direction]);
        visible: !isMobile && ma.containsMouse;
    }

    MouseArea {
        id: ma;
        anchors.fill: parent;
        cursorShape: Qt.PointingHandCursor;
        hoverEnabled: true;
        onClicked: root.direction = (root.direction + 1) % 4;
    }
}
