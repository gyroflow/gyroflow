// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC

QQC.ComboBox {
    id: root;

    //property alias icon: ti.icon;
    property alias itemHeight: pp.itemHeight;

    implicitWidth: 150 * dpiScale;
    height: 35 * dpiScale;

    font.pixelSize: 13 * dpiScale;
    font.family: styleFont;

    hoverEnabled: enabled;

    delegate: pp.contentItem.delegate;

    indicator: DropdownChevron { height: pp.itemHeight / 2.4; }

    background: Rectangle {
        color: root.hovered || root.activeFocus? Qt.lighter(styleButtonColor, 1.2) : styleButtonColor;
        opacity: root.down || !parent.enabled? 0.75 : 1.0;
        Ease on opacity { duration: 100; }
        radius: 6 * dpiScale;
        anchors.fill: parent;
        border.width: style === "light"? (1 * dpiScale) : 0;
        border.color: "#cccccc";
    }
    Keys.onPressed: (e) => {
        if (e.key == Qt.Key_Space) {
            root.focus = false;
            window.togglePlay();
            e.accepted = true;
        } else if (e.key == Qt.Key_Enter || e.key == Qt.Key_Return) {
            pp.open();
            pp.focus = true;
        }
    }

    scale: root.pressed? 0.98 : 1.0;
    Ease on scale {  }

    contentItem: Text {
        id: ti;
        text: qsTranslate("Popup", root.displayText);
        color: styleTextColor;
        font: root.font;
        anchors.left: parent.left;
        anchors.leftMargin: 10 * dpiScale;
        verticalAlignment: Text.AlignVCenter;
        opacity: parent.enabled? 1.0 : 0.5;
    }

    popup: Popup {
        id: pp;
        font: root.font;
        model: root.delegateModel;
        currentIndex: root.currentIndex;
        highlightedIndex: root.highlightedIndex;
    }

    property alias tooltip: tt.text;
    ToolTip { id: tt; visible: text.length > 0 && root.hovered; }
}
