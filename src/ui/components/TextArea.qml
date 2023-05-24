// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC

Flickable {
    id: view;
    height: 300 * dpiScale;
    width: 300 * dpiScale;
    property alias text: ta.text;
    property alias tooltip: tt.text;
    flickableDirection: Flickable.VerticalFlick;
    QQC.ScrollIndicator.vertical: QQC.ScrollIndicator { }
    clip: true;

    QQC.TextArea.flickable: QQC.TextArea {
        id: ta;
        selectionColor: styleAccentColor;
        placeholderTextColor: Qt.darker(styleTextColor);
        padding: 0;
        selectByMouse: true;
        topPadding: 10 * dpiScale;
        Component.onCompleted: Qt.callLater(view.returnToBounds);

        color: styleTextColor;
        opacity: enabled? 1.0 : 0.5;
        font.family: styleFont;
        font.pixelSize: 14 * dpiScale;
        background: Rectangle {
            radius: 5 * dpiScale;
            color: ta.activeFocus? styleBackground2 : styleButtonColor;
            border.color: styleButtonColor;
            border.width: 1 * dpiScale;

            opacity: ta.hovered && !ta.activeFocus? 0.8 : 1.0;

            Rectangle {
                layer.enabled: true;

                width: parent.width - 2*x;
                height: 6 * dpiScale;
                color: ta.activeFocus? styleAccentColor : "#9a9a9a";
                anchors.bottom: parent.bottom;
                anchors.bottomMargin: -1 * dpiScale;
                radius: parent.radius;

                Rectangle {
                    width: parent.width;
                    height: (ta.activeFocus? 4 : 5) * dpiScale;
                    color: ta.activeFocus? styleBackground2 : styleButtonColor;
                    y: 0;
                }
            }
        }
    }
    ToolTip { id: tt; visible: text.length > 0 && ta.hovered; }
}
