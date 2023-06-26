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
    property bool opened: col.children.length > 0;
    property alias loader: loader.active;
    property alias loaderProgress: loader.progress;
    property alias spacing: col.spacing;
    property alias innerItem: innerItem;
    default property alias data: col.data;
    property string iconName;
    property bool canEnsureVisible: false;

    Component.onCompleted: {
        const val = window.settings.value(root.objectName + "-opened", root.opened);
        root.opened = (val == true || val == 1 || val == "true");
    }

    function ensureVisible() {
        const flick = parent.parent.parent.parent;
        if (canEnsureVisible && opened && anim.enabled && (parent.y + height > flick.height)) {
            flick.contentY = parent.y;
        }
    }

    width: parent.width;
    height: btn.height + (opened? col.height : 0);
    Ease on height { id: anim; }
    onHeightChanged: Qt.callLater(root.ensureVisible);
    clip: true;
    onOpenedChanged: {
        anim.enabled = true;
        timer.start();
    }
    Timer {
        id: timer;
        interval: 700;
        onTriggered: { anim.enabled = false; canEnsureVisible = true; }
    }

    QQC.Button {
        id: btn;

        icon.name: iconName || "";
        icon.source: iconName ? "qrc:/resources/icons/svg/" + iconName + ".svg" : "";

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

        background: Item {
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
        onClicked: {
            if (col.children.length > 0) {
                root.opened = !root.opened;
                canEnsureVisible = true;
                window.settings.setValue(root.objectName + "-opened", root.opened);
            } else {
                root.clicked();
            }
        }

        Keys.onPressed: (e) => {
            if (e.key == Qt.Key_Enter || e.key == Qt.Key_Return) {
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
            bottomPadding: 20 * dpiScale;
        }
    }
    LoaderOverlay { id: loader; }
}
