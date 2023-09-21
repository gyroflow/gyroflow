// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

import QtQuick
import "."

LinkButton {
    visible: Qt.platform.os == "android" || Qt.platform.os == "ios";
    width: 60 * dpiScale;
    height: width;
    anchors.right: parent.right;
    anchors.top: parent.top;
    textColor: styleTextColor;
    iconName: "close";
    icon.width: 25 * dpiScale;
    icon.height: 25 * dpiScale;
    leftPadding: 0;
    rightPadding: 0;
    topPadding: 10 * dpiScale;
    Component.onCompleted: { background.color = "#80000000"; background.radius = 10 * dpiScale; }
}
