// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick

Rectangle {
    id: root;
    property real trimStart: 0;
    property real trimEnd: 1.0;

    property real trimStartAdjustment: 0;
    property real trimEndAdjustment: 0;

    property bool active: rightTrimDrag.active || leftTrimDrag.active;

    x: parent.width * mapToVisibleArea(Math.max(0.0, trimStart + trimStartAdjustment));
    width: Math.max(10, parent.width * mapToVisibleArea(Math.min(1.0, trimEnd + trimEndAdjustment)) - x);
    color: "#19ffffff";
    border.width: 2 * dpiScale;
    border.color: styleAccentColor;
    radius: 3 * dpiScale;
    clip: true;
    function mapToVisibleArea(v: real): real { return parent.parent.parent.mapToVisibleArea(v); }
    function mapFromVisibleArea(v: real): real { return parent.parent.parent.mapFromVisibleArea(v); }
    property real visibleRange: (parent.parent.parent.visibleAreaRight - parent.parent.parent.visibleAreaLeft);

    signal changeTrimStart(real val);
    signal changeTrimEnd(real val);
    signal reset();

    Rectangle {
        color: parent.border.color;
        radius: parent.radius;
        height: parent.height;
        width: 5 * dpiScale;
        MouseArea {
            anchors.fill: parent;
            cursorShape: Qt.SizeHorCursor;
            onDoubleClicked: root.reset();
        }
        DragHandler {
            id: leftTrimDrag;
            target: null;
            onActiveChanged: if (!active) { root.changeTrimStart(Math.max(0.0, root.trimStart + root.trimStartAdjustment)); root.trimStartAdjustment = 0; }
            onActiveTranslationChanged: root.trimStartAdjustment = (leftTrimDrag.activeTranslation.x / root.parent.width) * root.visibleRange;
        }

        Rectangle {
            color: parent.color;
            width: 10 * dpiScale;
            height: 25 * dpiScale;
            rotation: 45;
            x: -2 * dpiScale;
            y: -width/2;
        }
    }
    Rectangle {
        anchors.right: parent.right;
        color: parent.border.color;
        radius: parent.radius;
        height: parent.height;
        width: 5 * dpiScale;
        MouseArea {
            anchors.fill: parent;
            cursorShape: Qt.SizeHorCursor;
            onDoubleClicked: root.reset();
        }
        DragHandler {
            id: rightTrimDrag;
            target: null;
            onActiveChanged: if (!active) { root.changeTrimEnd(Math.min(1.0, root.trimEnd + root.trimEndAdjustment)); root.trimEndAdjustment = 0; }
            onActiveTranslationChanged: root.trimEndAdjustment = (rightTrimDrag.activeTranslation.x / root.parent.width) * root.visibleRange;
        }
        Rectangle {
            color: parent.color;
            width: 10 * dpiScale;
            height: 25 * dpiScale;
            rotation: 45;
            x: -2 * dpiScale;
            y: parent.height - height + width/2;
        }
    }
}
