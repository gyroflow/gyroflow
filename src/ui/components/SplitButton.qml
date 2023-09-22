// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2023 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC

Rectangle {
    id: root;
    property string iconName;

    // TODO popup direction
    property alias text: mainbtn.text;
    property alias model: popup.model;
    property alias popup: popup;
    property alias btn: mainbtn;
    property alias splitbtn: splitbtn;

    property bool isDown: false;

    width: mainbtn.width;
    height: mainbtn.height;
    layer.enabled: true;
    opacity: enabled? 1.0 : 0.6;
    Ease on opacity { }
    border.width: 1 * dpiScale;
    border.color: Qt.darker(styleAccentColor, 1.5);
    radius: 6 * dpiScale;
    color: "transparent";

    function open() {
        const pt = window.mapFromItem(root, 0, 0);
        popup.x = pt.x - popup.width + width;
        popup.y = pt.y + (isDown? height : -popup.height);
        popup.open();
    }

    Button {
        id: mainbtn;
        icon.name: root.iconName || "";
        icon.source: root.iconName ? "qrc:/resources/icons/svg/" + root.iconName + ".svg" : "";

        rightPadding: 47 * dpiScale;
        fadeWhenDisabled: root.enabled;
        Ease on opacity { }
    }
    Button {
        id: splitbtn;
        textColor: mainbtn.textColor;
        anchors.right: mainbtn.right;
        width: 35 * dpiScale;
        height: mainbtn.height;
        contentItem: Item { }
        accent: mainbtn.accent;
        fadeWhenDisabled: root.enabled;

        DropdownChevron { opened: popup.visible; color: mainbtn.textColor; anchors.centerIn: parent; }
        onClicked: root.open();
    }
    Rectangle {
        anchors.left: splitbtn.left;
        width: 1 * dpiScale;
        height: mainbtn.height;
        color: Qt.darker(styleAccentColor, 1.5);
    }
    Popup {
        id: popup;
        parent: window;
        x: -width + root.width;
        y: -height - 5 * dpiScale;
        width: Math.max(root.width, popup.maxItemWidth + 10 * dpiScale);
        currentIndex: -1;
    }
}
