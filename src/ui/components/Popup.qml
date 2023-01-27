// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC
import QtQuick.Controls.impl as QQCI
import QtQuick.Controls.Material.impl as QQCMI

QQC.Popup {
    id: popup;
    width: parent.width;
    implicitHeight: (lv.count * itemHeight) + 4 * dpiScale;
    padding: 2 * dpiScale;
    property alias model: lv.model;
    property alias currentIndex: lv.currentIndex;
    property alias lv: lv;
    property int highlightedIndex: currentIndex;
    property real itemHeight: 35 * dpiScale;
    font.pixelSize: 12 * dpiScale;
    font.family: styleFont;
    property real maxItemWidth: 0;

    property var icons: [];
    property var colors: [];

    signal clicked(int index);

    Component {
        id: dlgC;
        QQC.ItemDelegate {
            id: dlg;
            width: parent? parent.width : 0;
            implicitHeight: popup.itemHeight;

            contentItem: QQCI.IconLabel {
                anchors.fill: parent;
                text: qsTr(modelData);
                icon.name: popup.icons[index] || "";
                icon.source: popup.icons[index] ? "qrc:/resources/icons/svg/" + popup.icons[index] + ".svg" : "";
                icon.color: c;
                icon.height: popup.itemHeight / 2 + 5 * dpiScale;
                icon.width: popup.itemHeight / 2 + 5 * dpiScale;
                alignment: Qt.AlignLeft;
                leftPadding: 12 * dpiScale;
                rightPadding: 12 * dpiScale;
                color: c;
                property color c: popup.colors[index] || styleTextColor;
                topPadding: popup.itemHeight / 3.5;
                bottomPadding: popup.itemHeight / 3.5;

                font: popup.font;
                onImplicitWidthChanged: { if (implicitWidth > popup.maxItemWidth) popup.maxItemWidth = implicitWidth; }
            }

            scale: dlg.down? 0.970 : 1.0;
            Ease on scale { }

            MouseArea { anchors.fill: parent; acceptedButtons: Qt.NoButton; cursorShape: Qt.PointingHandCursor; }

            function clickHandler() {
                popup.focus = false;
                popup.parent.focus = true;
                popup.clicked(index);
                popup.visible = false;
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
            highlighted: popup.highlightedIndex === index;
        }
    }

    contentItem: ListView {
        id: lv;
        clip: true;
        QQC.ScrollIndicator.vertical: QQC.ScrollIndicator { }
        delegate: dlgC;
        highlightFollowsCurrentItem: true;
        focus: true;
        keyNavigationEnabled: true;
        highlight: Rectangle {
            color: styleHighlightColor;
            radius: 4 * dpiScale;
        }
    }

    background: Rectangle {
        color: styleButtonColor;
        border.width: 1 * dpiScale;
        border.color: stylePopupBorder
        radius: 4 * dpiScale;
        layer.enabled: true;
        layer.effect: QQCMI.ElevationEffect { elevation: 8 }
    }
}
