// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC

Flickable {
    property alias inner: tabColInner;
    default property alias data: tabColInner.data;
    property real parentHeight: 0;

    function updateHeight(tabBarHeight: real): void {
        height = Qt.binding(() => Math.min(tabColInner.height, parentHeight - tabBarHeight));
    }

    width: parent.width;
    height: 0;
    clip: true;

    QQC.ScrollIndicator.vertical: QQC.ScrollIndicator { padding: 0; }

    contentHeight: tabColInner.height;
    contentWidth: width;
    Column {
        id: tabColInner;
        spacing: 5 * dpiScale;
        width: parent.width;
    }
}
