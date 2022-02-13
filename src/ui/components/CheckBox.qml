// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC

QQC.CheckBox {
    id: cb;
    onCheckedChanged: if (checked) { cm1.width = 0; cm2.width = 0; cbanim.start(); }
    implicitHeight: 30 * dpiScale;

    Keys.onPressed: (e) => {
        if (e.key == Qt.Key_Space) {
            root.focus = false;
            window.togglePlay();
            e.accepted = true;
        } else if (e.key == Qt.Key_Enter || e.key == Qt.Key_Return) {
            checked = !checked;
        }
    }

    indicator: Rectangle {
        implicitWidth: 20 * dpiScale
        implicitHeight: 20 * dpiScale
        x: cb.leftPadding
        y: parent.height / 2 - height / 2
        radius: 5 * dpiScale;
        color: cb.checked? styleAccentColor : "transparent";
        Behavior on color { ColorAnimation { duration: 300; easing.type: Easing.OutExpo; } }
        border.color: cb.checked? styleAccentColor : "#999999";

        opacity: cb.down || cb.activeFocus? 0.8 : 1.0;
        Ease on opacity { }

        Item {
            id: cm;
            anchors.fill: parent;
            visible: opacity > 0;
            Ease on opacity { }
            opacity: checked? 1 : 0;
            Rectangle {
                id: cm1;
                width: 0;
                height: 2 * dpiScale;
                radius: height;
                color: styleTextColorOnAccent;
                x: 6.5 * dpiScale;
                y: 9 * dpiScale;
                transformOrigin: Item.Left;
                transform: Rotation { angle: 45; }
            }
            Rectangle {
                id: cm2;
                width: 0;
                height: 2 * dpiScale;
                color: styleTextColorOnAccent;
                radius: height;
                x: 7.5 * dpiScale;
                y: 13 * dpiScale;
                transformOrigin: Item.Left;
                transform: Rotation { angle: -45; }
            }
            ParallelAnimation {
                id: cbanim;
                loops: 1;
                PropertyAnimation { target: cm1; property: "width"; from: 0; to: 5 * dpiScale; easing.type: Easing.InOutCubic; duration: 200; }
                SequentialAnimation {
                    PauseAnimation { duration: 80; }
                    PropertyAnimation { target: cm2; property: "width"; from: 0; to: 10 * dpiScale; easing.type: Easing.InOutCubic; duration: 200; }
                }
            }
        }
    }
    topPadding: 0;
    bottomPadding: 0;
    leftPadding: 0;

    contentItem: Text {
        text: cb.text;
        font.pixelSize: 13 * dpiScale;
        font.family: styleFont;
        color: styleTextColor;
        opacity: enabled ? 1.0 : 0.3
        verticalAlignment: Text.AlignVCenter
        leftPadding: cb.indicator.width + cb.spacing
    }
    MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; acceptedButtons: Qt.NoButton; }

    property alias tooltip: tt.text;
    ToolTip { id: tt; visible: text.length > 0 && cb.hovered; }
}
