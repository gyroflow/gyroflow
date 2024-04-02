// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC
import QtQuick.Controls.impl as QQCI
import QtQuick.Controls.Material.impl as QQCMI

QQC.ItemDelegate {
    property QQC.Popup parentPopup: null;
    property Item lv: null;

    id: dlg;
    width: parent? parent.width : 0;
    implicitHeight: parentPopup.itemHeight;

    contentItem: QQCI.IconLabel {
        anchors.fill: parent;
        text: qsTranslate("Popup", modelData);
        icon.name: parentPopup.icons[index] || "";
        icon.source: parentPopup.icons[index] ? "qrc:/resources/icons/svg/" + parentPopup.icons[index] + ".svg" : "";
        icon.color: c;
        icon.height: parentPopup.itemHeight / 2 + 5 * dpiScale;
        icon.width: parentPopup.itemHeight / 2 + 5 * dpiScale;
        alignment: Qt.AlignLeft;
        leftPadding: 12 * dpiScale;
        rightPadding: 12 * dpiScale;
        color: c;
        property color c: parentPopup.colors[index] || styleTextColor;
        topPadding: parentPopup.itemHeight / 3.5;
        bottomPadding: parentPopup.itemHeight / 3.5;

        font: parentPopup.font;
        onImplicitWidthChanged: { if (implicitWidth > parentPopup.maxItemWidth) parentPopup.maxItemWidth = implicitWidth; }
    }

    scale: dlg.down? 0.970 : 1.0;
    Ease on scale { }

    MouseArea { anchors.fill: parent; acceptedButtons: Qt.NoButton; cursorShape: Qt.PointingHandCursor; }

    function clickHandler(): void {
        parentPopup.focus = false;
        parentPopup.parent.focus = true;
        parentPopup.clicked(index);
        parentPopup.visible = false;
    }

    onClicked: clickHandler();

    Keys.onPressed: (e) => {
        if (e.key == Qt.Key_Enter || e.key == Qt.Key_Return) {
            clickHandler();
        }
    }

    background: Rectangle {
        color: dlg.checked? styleAccentColor : (dlg.hovered || dlg.highlighted? styleHighlightColor : "transparent");
        anchors.fill: parent;
        anchors.margins: 2 * dpiScale;
        radius: 4 * dpiScale;
        opacity: dlg.checked? 0.5 : 1.0;

        Rectangle {
            x: 1 * dpiScale;
            color: styleAccentColor;
            height: parent.height * 0.45;
            width: 3 * dpiScale;
            radius: width;
            y: (parent.height - height) / 2;
            visible: lv.currentIndex === index;
        }
    }
    highlighted: parentPopup.highlightedIndex === index;
}
