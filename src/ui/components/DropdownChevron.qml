// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC
import QtQuick.Controls.impl as QQCI

QQCI.IconImage {
    property bool opened: false;
    anchors.right: parent.right;
    anchors.rightMargin: 8 * dpiScale;
    rotation: opened? 180 : 0;
    Ease on rotation { }

    width: height;
    height: 15 * dpiScale;
    anchors.verticalCenter: parent.verticalCenter;
    name: "chevron-down";
    color: styleTextColor;
    layer.enabled: true;
    layer.textureSize: Qt.size(height*2, height*2);
    layer.smooth: true;
    anchors.alignWhenCentered: false
    // Image {
    //     source: "../../../resources/icons/svg/chevron-down-2.svg";
    //     sourceSize.width: parent.height;
    //     anchors.centerIn: parent;
    // }
}
/*
Text {
    text: "\uE971"
    color: styleTextColor;
    property bool opened: false;
    anchors.right: parent.right;
    anchors.rightMargin: 8 * dpiScale;
    rotation: opened? 0 : 180;
    Ease on rotation { }
    font.family: "Segoe MDL2 Assets";
    font.pixelSize: 10 * dpiScale;
    verticalAlignment: Text.AlignVCenter;
    height: parent.height;
}
*/
