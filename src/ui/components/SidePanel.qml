// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC

ResizablePanel {
    height: parent? parent.height - y : 0;
    default property alias data: col.data;
    property alias col: col;
    defaultWidth: 340 * dpiScale;
    property real topPadding: 0;
    property real bottomPadding: 0;

    Flickable {
        width: parent.width - 2*x;
        x: 4 * dpiScale;
        y: topPadding;
        height: parent.height - y - parent.bottomPadding;
        clip: true;
        QQC.ScrollIndicator.vertical: QQC.ScrollIndicator { padding: 0; }

        contentHeight: col.height;
        contentWidth: width;
        Column {
            id: col;
            spacing: 5 * dpiScale;
            width: parent.width;
        }
    }
}
