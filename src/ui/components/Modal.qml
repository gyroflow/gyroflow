// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC
import QtQuick.Controls.impl as QQCI

Rectangle {
    id: root;
    property alias t: t;
    property alias textFormat: t.textFormat;
    property alias text: t.text;
    property alias buttons: btns.model;
    property alias mainColumn: mainColumn;
    property bool opened: false;
    property int accentButton: -1;
    onTextChanged: {
        if (text.indexOf("<") > -1 && textFormat != Text.MarkdownText) {
            text = text.replace(/\n/g, "<br>");
            textFormat = Text.StyledText;
        }
    }

    enum IconType { NoIcon, Info, Warning, Error, Success, Question }

    property int iconType: Modal.Warning;

    signal clicked(int index);

    anchors.fill: parent;
    color: "#80000000";
    opacity: pp.opacity;
    visible: opacity > 0;
    onVisibleChanged: {
        if (visible && iconType != Modal.NoIcon) {
            switch (iconType) {
                case Modal.Info:     icon.name = "info";      icon.color = styleAccentColor; break;
                case Modal.Warning:  icon.name = "warning";   icon.color = "#f6a10c"; break;
                case Modal.Error:    icon.name = "error";     icon.color = "#d82626"; break;
                case Modal.Success:  icon.name = "confirmed"; icon.color = "#3cc42f"; break;
                case Modal.Question: icon.name = "question";  icon.color = styleAccentColor; break;
            }
            icon.visible = true;
            ease.enabled = false;
            icon.scale = 0.4;
            ease.enabled = true;
            icon.scale = 1;
        }
    }

    MouseArea { visible: root.opened; anchors.fill: parent; preventStealing: true; hoverEnabled: true; }
    Rectangle {
        id: pp;
        anchors.centerIn: parent;
        anchors.verticalCenterOffset: root.opened? 0 : -50 * dpiScale;
        Ease on anchors.verticalCenterOffset { }
        Ease on opacity { }
        opacity: root.opened? 1 : 0;
        width: Math.min(window.width * 0.95, Math.max(btnsRow.width + 100 * dpiScale, root.text.length > 200? parent.width * 0.8 : 400 * dpiScale));
        height: col.height + 30 * dpiScale;
        property real offs: 0;
        color: styleBackground2;
        radius: 7 * dpiScale;
        border.width: 1 * dpiScale;
        border.color: styleSliderHandle;

        Column {
            id: col;
            width: parent.width;
            anchors.centerIn: parent;

            QQCI.IconImage {
                id: icon;
                visible: false;
                color: styleTextColor;
                height: 70 * dpiScale;
                width: height;
                anchors.horizontalCenter: parent.horizontalCenter;
                layer.enabled: true;
                layer.textureSize: Qt.size(height*2, height*2);
                layer.smooth: true;
                Ease on scale { id: ease; type: Easing.OutElastic; duration: 1000; }
            }

            Item { height: 10 * dpiScale; width: 1; }
            Flickable {
                width: parent.width;
                height: Math.min(contentHeight, root.height - icon.height - btnsRow.height - 150 * dpiScale);
                contentWidth: width;
                contentHeight: mainColumn.height;
                clip: true;
                QQC.ScrollBar.vertical: QQC.ScrollBar { }
                Column {
                    id: mainColumn;
                    x: 15 * dpiScale;
                    width: parent.width - 2*x;
                    spacing: 10 * dpiScale;
                    BasicText {
                        id: t;
                        width: parent.width;
                        horizontalAlignment: Text.AlignHCenter;
                        wrapMode: Text.WordWrap;
                        font.pixelSize: 14 * dpiScale;

                        MouseArea {
                            anchors.fill: parent;
                            cursorShape: parent.hoveredLink? Qt.PointingHandCursor : Qt.ArrowCursor;
                            acceptedButtons: Qt.NoButton;
                        }
                    }
                }
            }
            Item { height: 25 * dpiScale; width: 1; }
            Flow {
                id: btnsRow;
                anchors.horizontalCenter: parent.horizontalCenter;
                spacing: 10 * dpiScale;
                onWidthChanged: {
                    Qt.callLater(() => {
                        if (btnsRow.width > parent.width - 20 * dpiScale) {
                            btnsRow.width = parent.width - 20 * dpiScale;
                        }
                    });
                }
                Repeater {
                    id: btns;
                    Button {
                        text: modelData;
                        onClicked: root.clicked(index);
                        leftPadding: 20 * dpiScale;
                        rightPadding: 20 * dpiScale;
                        accent: root.accentButton === index;
                    }
                }
            }
        }
    }
}
