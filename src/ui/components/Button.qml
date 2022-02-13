// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC
import QtQuick.Controls.Material as QQCM
import QtQuick.Controls.Material.impl as QQCMI

QQC.Button {
    id: root;

    property bool accent: false;
    property color textColor: root.accent? styleTextColorOnAccent : styleTextColor;
    QQCM.Material.foreground: textColor;
    Component.onCompleted: {
        if (contentItem.color) {
            contentItem.color = Qt.binding(() => root.textColor);
            icon.color = Qt.binding(() => root.textColor);
            if (fadeWhenDisabled) {
                contentItem.opacity = Qt.binding(() => !root.enabled? 0.75 : 1.0);
            }
        }
    }

    property bool fadeWhenDisabled: true;

    height: 35 * dpiScale;
    leftPadding: 15 * dpiScale;
    rightPadding: 15 * dpiScale;
    topPadding: 8 * dpiScale;
    bottomPadding: 8 * dpiScale;
    font.pixelSize: 14 * dpiScale;
    font.family: styleFont;
    hoverEnabled: enabled;

    background: Rectangle {
        color: root.accent? root.hovered || root.activeFocus? Qt.lighter(styleAccentColor, 1.1) : styleAccentColor : root.hovered || root.activeFocus? Qt.lighter(styleButtonColor, 1.2) : styleButtonColor;
        opacity: !parent.enabled && fadeWhenDisabled? 0.75 : root.down? 0.75 : 1.0;
        Ease on opacity { duration: 100; }
        radius: 6 * dpiScale;
        anchors.fill: parent;
        border.width: style === "light"? (1 * dpiScale) : 0;
        border.color: "#cccccc";
    }

    scale: root.down? 0.970 : 1.0;
    Ease on scale { }
    font.capitalization: Font.Normal;

    property alias tooltip: tt.text;
    ToolTip { id: tt; visible: text.length > 0 && root.hovered; }
    
    Keys.onPressed: (e) => {
        if (e.key == Qt.Key_Space) {
            root.focus = false;
            window.togglePlay();
            e.accepted = true;
        } else if (e.key == Qt.Key_Enter || e.key == Qt.Key_Return) {
            root.clicked();
        }
    }
}
