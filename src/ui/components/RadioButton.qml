// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC

QQC.RadioButton {
    id: root;
    implicitHeight: 30 * dpiScale;

    indicator: Rectangle {
        implicitWidth: 20 * dpiScale;
        implicitHeight: 20 * dpiScale;
        radius: width;
        color: "transparent";
        border.width: 1.5 * dpiScale;
        border.color: root.down || root.checked ? Qt.darker(styleAccentColor, 1.2) : styleSliderHandle;
        Behavior on border.color { ColorAnimation { duration: 500; easing.type: Easing.OutExpo; } }

        x: root.leftPadding;
        y: (parent.height - height) / 2;

        Rectangle {
            anchors.centerIn: parent;
            width: parent.width * 0.6;
            height: width;
            radius: width;
            color: styleAccentColor;
            scale: root.checked? 1 : 0;
            opacity: root.checked? 1 : 0;
            visible: opacity > 0;
            Ease on scale { }
            Ease on opacity { }
        }
    }
    contentItem: Text {
        text: root.text;
        font.pixelSize: 13 * dpiScale;
        font.family: styleFont;
        color: styleTextColor;
        opacity: enabled ? 1.0 : 0.3;
        linkColor: styleAccentColor;
        leftPadding: root.indicator.width + root.spacing;
        verticalAlignment: Text.AlignVCenter;
    }

    property alias tooltip: tt.text;
    ToolTip { id: tt; visible: text.length > 0 && cb.hovered; }
}
