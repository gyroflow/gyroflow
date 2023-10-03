// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC

QQC.ToolTip {
    id: root;
    delay: 500;
    property real offsetY: 0;
    background: Rectangle {
        color: styleButtonColor;
        border.width: 1 * dpiScale;
        border.color: stylePopupBorder
        radius: 4 * dpiScale;
    }
    enter: Transition {
        NumberAnimation { property: "y"; from: -height/1.3 - bottomMargin + offsetY; to: -height - bottomMargin + offsetY; easing.type: Easing.OutExpo; duration: 500; }
        NumberAnimation { property: "opacity"; from: 0.0; to: 1.0; easing.type: Easing.OutExpo; duration: 500; }
    }
    exit: Transition {
        NumberAnimation { property: "y"; from: -height - bottomMargin + offsetY; to: -height/1.3 - bottomMargin + offsetY; easing.type: Easing.OutExpo; duration: 500; }
        NumberAnimation { property: "opacity"; from: 1.0; to: 0.0; easing.type: Easing.OutExpo; duration: 500; }
    }
    contentItem: Text {
        font.pixelSize: 12 * dpiScale;
        text: root.text;
        color: styleTextColor;
    }
    bottomMargin: 5 * dpiScale;
    topPadding: 5 * dpiScale;
    rightPadding: 8 * dpiScale;
    bottomPadding: topPadding;
    leftPadding: rightPadding;
    enabled: !isMobile;
}
