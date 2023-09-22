// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2023 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC
import QtQuick.Controls.Material as QQCM
import QtQuick.Layouts

Column {
    id: root;
    width: parent.width;
    default property alias data: stackLayout.data;
    property var tabs: [];
    property var tabsIcons: [];
    property var tabsIconsSize: [];
    function updateHeights() { Qt.callLater(stackLayout.updateHeights); }
    onWidthChanged: root.updateHeights();
    property alias currentIndex: tabBar.currentIndex;
    QQC.TabBar {
        id: tabBar;
        width: parent.width;
        font.family: styleFont;
        font.pixelSize: 13 * dpiScale;
        font.bold: true;
        implicitHeight: 40 * dpiScale;
        currentIndex: 0;
        background: Rectangle {
            color: styleBackground2;
            radius: 5 * dpiScale;
            Rectangle { width: parent.width; height: 1 * dpiScale; color: stylePopupBorder; anchors.bottom: parent.bottom; }
        }
        onCurrentIndexChanged: root.updateHeights();
        Repeater {
            model: root.tabs;
            QQC.TabButton {
                text: qsTr(modelData);
                QQCM.Material.foreground: styleTextColor;
                font: tabBar.font;
                implicitHeight: tabBar.implicitHeight;
                icon.name: root.tabsIcons[index];
                icon.width: root.tabsIconsSize[index] * dpiScale;
                icon.height: root.tabsIconsSize[index] * dpiScale;
                padding: 0;
                MouseArea { anchors.fill: parent; acceptedButtons: Qt.NoButton; cursorShape: Qt.PointingHandCursor; }
                Component.onCompleted: root.updateHeights();
            }
        }
    }
    StackLayout {
        id: stackLayout;
        y: tabBar.height;
        width: parent.width;
        currentIndex: tabBar.currentIndex;
        visible: tabBar.visible;
        function updateHeights() {
            for (let i = 0; i < children.length; ++i) children[i].updateHeight(tabBar.height);
        }
    }
}
