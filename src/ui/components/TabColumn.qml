// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC

Flickable {
    width: parent.width;
    property real parentHeight: 0;
    height: Math.min(tabColInner.height, parentHeight);
    clip: true;
    QQC.ScrollIndicator.vertical: QQC.ScrollIndicator { padding: 0; }
    property alias inner: tabColInner;
    default property alias data: tabColInner.data;
    contentHeight: tabColInner.height;
    contentWidth: width;
    function updateHeight() {
        height = Qt.binding(() => Math.min(tabColInner.height, parentHeight));
    }
    Column {
        id: tabColInner;
        spacing: 5 * dpiScale;
        width: parent.width;
    }
}
