// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC

QQC.AbstractButton {
    id: root;
    height: 25 * dpiScale;
    width: 25 * dpiScale;
    font.pixelSize: 10 * dpiScale;
    font.family: styleFont;

    background: Rectangle {
        color: if (style === "light") {
                   return root.checked? root.hovered || root.activeFocus ? Qt.lighter(styleAccentColor, 1.1) : Qt.lighter(styleAccentColor) : root.hovered || root.activeFocus? Qt.darker(styleButtonColor, 1.2) : styleHrColor;
               } else {
                   return root.checked? root.activeFocus ? Qt.darker(styleAccentColor, 1.1) : Qt.darker(styleAccentColor) : root.hovered || root.activeFocus? Qt.lighter(styleButtonColor, 1.2) : Qt.darker(styleButtonColor, 1.2);
               }

        opacity: root.down || !parent.enabled? 0.75 : 1.0;
        Ease on opacity { duration: 100; }
        radius: 3 * dpiScale;
    }

    contentItem: BasicText {
        text: root.text;
        color: style === "light"? (root.checked? styleTextColorOnAccent : styleTextColor) : styleTextColor;
        opacity: root.checked? 1.0 : 0.5;
        font: root.font;
        leftPadding: 0;
        horizontalAlignment: Text.AlignHCenter;
        verticalAlignment: Text.AlignVCenter;
    }
    onClicked: checked = !checked;

    Keys.onPressed: (e) => {
        if (e.key == Qt.Key_Space) {
            root.focus = false;
            window.togglePlay();
            e.accepted = true;
        } else if (e.key == Qt.Key_Enter || e.key == Qt.Key_Return) {
            checked = !checked;
        }
    }
}
