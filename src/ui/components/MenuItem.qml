// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC
import QtQuick.Controls.impl as QQCI
import QtQuick.Controls.Material as QQCM

Item {
    id: root;
    signal clicked();
    property alias text: btn.text;
    property alias icon: btn.icon.name;
    property bool opened: col.children.length;
    property alias loader: loader.active;
    property alias loaderProgress: loader.progress;
    property alias spacing: col.spacing;
    property alias innerItem: innerItem;
    default property alias data: col.data;

    width: parent.width;
    height: btn.height + (opened? col.height : 0);
    Ease on height { id: anim; }
    clip: true;
    onOpenedChanged: {
        anim.enabled = true;
        timer.start();
    }
    Timer {
        id: timer;
        interval: 700;
        onTriggered: anim.enabled = false;
    }

    QQC.Button {
        id: btn;
        width: parent.width;
        height: 36 * dpiScale;
        hoverEnabled: true;

        leftPadding: 8 * dpiScale;
        rightPadding: 0;
        topPadding: 0;
        bottomPadding: 0;
        Component.onCompleted: {
            if (contentItem.color) {
                contentItem.color = Qt.binding(() => styleTextColor);
                contentItem.icon.color = Qt.binding(() => styleTextColor);
            }
            contentItem.alignment = Qt.AlignLeft;
        }

        font.pixelSize: 14 * dpiScale;
        font.family: styleFont;
        font.capitalization: Font.Normal

        background: Rectangle {
            color: parent.down? Qt.darker(styleButtonColor, 1.1) : parent.hovered || parent.activeFocus? styleButtonColor : "transparent";
            radius: 5 * dpiScale;
            anchors.fill: parent;

            Rectangle {
                color: styleAccentColor;
                height: parent.height * 0.45;
                width: 3 * dpiScale;
                radius: width;
                opacity: root.opened? 1 : 0;
                y: root.opened? (parent.height - height) / 2 : -height;

                Ease on opacity { }
                Ease on y { }
            }

            MouseArea { anchors.fill: parent; acceptedButtons: Qt.NoButton; cursorShape: Qt.PointingHandCursor; }
        }

        DropdownChevron { visible: col.children.length > 0; opened: root.opened; anchors.rightMargin: 5 * dpiScale; }
        onClicked: if (col.children.length > 0) { root.opened = !root.opened; } else { root.clicked(); }

        Keys.onPressed: (e) => {
            if (e.key == Qt.Key_Space) {
                root.focus = false;
                window.togglePlay();
                e.accepted = true;
            } else if (e.key == Qt.Key_Enter || e.key == Qt.Key_Return) {
                btn.clicked();
            }
        }
    }

    Item {
        id: innerItem;
        width: col.width;
        height: col.height;
        Column {
            id: col;
            y: btn.height;
            x: 15 * dpiScale;
            opacity: root.opened? 1 : 0;
            Ease on opacity { }
            visible: opacity > 0;
            width: root.width - 2*x;
            spacing: 10 * dpiScale;
            topPadding: 10 * dpiScale;
        }
    }
    LoaderOverlay { id: loader; }
}
