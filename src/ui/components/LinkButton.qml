// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC
import QtQuick.Controls.Material as QQCM

QQC.Button {
    id: root;

    property color textColor: styleAccentColor;
    QQCM.Material.foreground: textColor;

    font.pixelSize: 12 * dpiScale;
    font.family: styleFont;
    font.underline: true;
    font.capitalization: Font.Normal
    hoverEnabled: enabled;
    property bool transparent: false;

    leftPadding: 15 * dpiScale;
    rightPadding: 15 * dpiScale;
    topPadding: 4 * dpiScale;
    bottomPadding: 5 * dpiScale;
    opacity: root.hovered && transparent? 0.8 : 1.0;

    background: Rectangle {
        color: (root.hovered || root.activeFocus) && !root.transparent? Qt.lighter(styleButtonColor, 1.2) : "transparent";
        opacity: root.down || !parent.enabled? 0.1 : 0.3;
        Ease on opacity { duration: 100; }
        radius: 5 * dpiScale;
        anchors.fill: parent;
    }

    MouseArea { anchors.fill: parent; acceptedButtons: Qt.NoButton; cursorShape: Qt.PointingHandCursor; }

    property alias tooltip: tt.text;
    ToolTip { id: tt; visible: text.length > 0 && root.hovered; }

    property string iconName;
    icon.name: iconName || "";
    icon.source: iconName ? "qrc:/resources/icons/svg/" + iconName + ".svg" : "";

    Keys.onReturnPressed: checked = !checked;
    Keys.onEnterPressed: checked = !checked;
}
