// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026

import QtQuick

Rectangle {
    id: root;

    property int viewMode: -1;
    property real visibleAreaLeft: 0.0;
    property real visibleAreaRight: 1.0;
    property real durationMs: 0.0;
    property real selectionStart: -1.0;
    property real selectionEnd: -1.0;
    property bool selecting: false;
    readonly property bool hasSelection: selectionStart >= 0.0 && selectionEnd >= 0.0 && Math.abs(selectionEnd - selectionStart) > 0.0001;
    readonly property real fadeRange: durationMs > 0.0 ? Math.min(200.0 / durationMs, Math.abs(selectionEnd - selectionStart) * 0.15) : 0.0;
    readonly property real fadeWidth: parent ? parent.width * fadeRange / Math.max(0.000001, visibleAreaRight - visibleAreaLeft) : 0.0;

    function selectionLeft(): real { return Math.min(selectionStart, selectionEnd); }
    function selectionRight(): real { return Math.max(selectionStart, selectionEnd); }
    function mapToVisibleArea(pos: real): real { return (pos - visibleAreaLeft) / (visibleAreaRight - visibleAreaLeft); }
    function begin(pos: real): void {
        selecting = true;
        selectionStart = pos;
        selectionEnd = pos;
    }
    function extendTo(pos: real): void { selectionEnd = pos; }
    function finish(): void { selecting = false; }
    function clear(): void {
        selectionStart = -1.0;
        selectionEnd = -1.0;
    }

    visible: viewMode === 3 && hasSelection;
    x: parent.width * mapToVisibleArea(selectionLeft());
    width: Math.max(1, parent.width * (mapToVisibleArea(selectionRight()) - mapToVisibleArea(selectionLeft())));
    color: style === "light" ? "#402080ff" : "#4050a0ff";
    border.color: style === "light" ? "#6090c0ff" : "#80b0ffff";
    border.width: 1;
    z: 2;
    opacity: selecting ? 0.9 : 0.65;

    Rectangle {
        width: Math.max(0.0, Math.min(root.fadeWidth, root.x));
        height: parent.height;
        x: -width;
        color: root.color;
        opacity: 0.45;
    }
    Rectangle {
        width: Math.max(0.0, Math.min(root.fadeWidth, root.parent.width - root.x - root.width));
        height: parent.height;
        x: parent.width;
        color: root.color;
        opacity: 0.45;
    }
}
