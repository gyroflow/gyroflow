// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC
import QtQuick.Dialogs

import "."

Item {
    id: root;
    property bool shown: false;
    property bool canShow: true;
    property bool isBlack: false;
    property int hlines: 3;
    property int vlines: 3;
    anchors.fill: parent;
    Item {
        id: inner;
        anchors.fill: parent;
        visible: opacity > 0;
        opacity: root.shown && root.canShow? 0.9 : 0;
        Ease on opacity { }
        Row {
            anchors.fill: parent;
            spacing: (parent.width - root.vlines*2*dpiScale) / root.vlines;
            Item { width: 1; height: 1; }
            Repeater { model: root.vlines - 1; Rectangle { width: 2 * dpiScale; height: parent.height; color: root.isBlack? "#000" : "#fff"; } }
        }
        Column {
            anchors.fill: parent;
            spacing: (parent.height - root.hlines*2*dpiScale) / root.hlines;
            Item { width: 1; height: 1; }
            Repeater { model: root.hlines - 1; Rectangle { height: 2 * dpiScale; width: parent.width; color: root.isBlack? "#000" : "#fff"; } }
        }
    }

    ContextMenuMouseArea {
        underlyingItem: root;
        onContextMenu: (isHold, x, y) => menuLoader.popup(root, x, y);
    }

    ContextMenuLoader {
        id: menuLoader;
        sourceComponent: Component {
            Menu {
                font.pixelSize: 11.5 * dpiScale;
                Menu {
                    Component.onCompleted: this.setIcon("grid");
                    title: qsTr("Grid guide");
                    Action { text: qsTr("Enabled"); checkable: true; checked: root.shown; onTriggered: root.shown = checked; }
                    Menu {
                        Component.onCompleted: this.setIcon("pencil");
                        title: qsTr("Color");
                        Action { text: qsTr("White"); checked: !root.isBlack; checkable: true; onTriggered: root.isBlack = false; }
                        Action { text: qsTr("Black"); checked: root.isBlack; checkable: true; onTriggered: root.isBlack = true; }
                    }
                    Menu {
                        Component.onCompleted: this.setIcon("grid");
                        title: qsTr("Lines");
                        NumberField {
                            width: 80 * dpiScale;
                            height: 25 * dpiScale;
                            value: +window.settings.value("gridLines", "2");
                            from: 1;
                            to: 100;
                            onValueChanged: { root.hlines = value + 1; root.vlines = value + 1; window.settings.setValue("gridLines", value); }
                        }
                    }
                }
            }
        }
    }
}
