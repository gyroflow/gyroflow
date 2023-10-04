// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC
import QtQuick.Controls.Material as QQCM
import QtQuick.Controls.Material.impl as QQCMI

QQC.Popup {
    id: popup;
    width: parent.width;
    implicitHeight: (Math.min(8, model[tabs[currentTab]].length) * itemHeight) + tabBar.height +  4 * dpiScale;
    padding: 2 * dpiScale;
    property var model: {};
    property real itemHeight: 35 * dpiScale;
    font.pixelSize: 12 * dpiScale;
    font.family: styleFont;
    onModelChanged: tabs = Object.keys(model);
    property int currentTab: 0;
    property var formatItem: item => item;
    property bool editable: false;
    property string editTooltip;

    property var tabs: [];
    property var icons: [];
    property var colors: [];

    function resetTab() {
        tabBar.currentIndex = 1;
        tabBar.currentIndex = 0;
    }

    signal clicked(int index);
    signal edit();

    contentItem: Column {
        width: popup.width;

        QQC.TabBar {
            id: tabBar;
            width: parent.width;
            font: popup.font;
            implicitHeight: popup.itemHeight;
            currentIndex: 0;

            background: Rectangle {
                color: styleButtonColor;
                Rectangle { width: parent.width; height: 1 * dpiScale; color: stylePopupBorder; anchors.bottom: parent.bottom; }
            }
            onCurrentIndexChanged: if (!popup.editable || currentIndex != popup.tabs.length) popup.currentTab = currentIndex;
            Repeater {
                model: popup.tabs;
                QQC.TabButton {
                    Component.onCompleted: Qt.callLater(popup.resetTab);
                    text: qsTr(modelData);
                    font: popup.font;
                    implicitHeight: popup.itemHeight;
                    padding: 0;
                }
            }

            QQC.TabButton {
                visible: popup.editable;
                implicitHeight: popup.itemHeight;
                padding: 0;
                MouseArea { anchors.fill: parent; acceptedButtons: Qt.NoButton; cursorShape: Qt.PointingHandCursor; }
                ToolTip {visible: !isMobile && text.length > 0 && parent.hovered; text: popup.editTooltip; }

                QQCM.Material.foreground: styleAccentColor;
                icon.name: "pencil";
                icon.source: "qrc:/resources/icons/svg/pencil.svg";
                icon.width: height * 0.45;
                icon.height: height * 0.45;
                onClicked: popup.edit();
                Keys.onReturnPressed: popup.edit();
                Keys.onEnterPressed: popup.edit();
                opacity: activeFocus? 0.8 : 1;
            }
        }
        StackLayout {
            id: stackLayout;
            width: parent.width
            currentIndex: popup.currentTab
            Repeater {
                model: popup.tabs;
                ListView {
                    Component.onCompleted: Qt.callLater(popup.resetTab);
                    id: tabLv;
                    clip: true;
                    height: (Math.min(8, model.length) * popup.itemHeight)
                    QQC.ScrollIndicator.vertical: QQC.ScrollIndicator { }
                    delegate: PopupDelegate { parentPopup: popup; lv: Item { property int currentIndex: -1; } }
                    onVisibleChanged: if (visible) focus = true;
                    model: popup.model[modelData].map(popup.formatItem);
                    keyNavigationEnabled: true;
                    highlight: Item { }
                }
            }
        }
    }

    background: Rectangle {
        color: styleButtonColor;
        border.width: 1 * dpiScale;
        border.color: stylePopupBorder;
        radius: 4 * dpiScale;
        layer.enabled: true;
        layer.effect: QQCMI.ElevationEffect { elevation: 8 }
    }
}
