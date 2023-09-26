// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

import QtQuick

MouseArea {
    id: root;
    anchors.fill: parent;
    acceptedButtons: Qt.RightButton;
    propagateComposedEvents: true;
    signal contextMenu(bool isHold, real x, real y);

    property Item underlyingItem: null;

    onClicked: mouse => { if (mouse.button === Qt.RightButton) root.contextMenu(false, mouse.x, mouse.y); }

    TapHandler {
        parent: root.underlyingItem || root.parent;
        acceptedDevices: PointerDevice.TouchScreen;
        onLongPressed: root.contextMenu(true, point.position.x, point.position.y);
    }
}
