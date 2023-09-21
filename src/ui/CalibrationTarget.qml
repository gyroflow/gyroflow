// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Window

import "components/"

Window {
    id: root;
    width: 1000;
    height: 571;
    minimumWidth: 900 * dpiScale;
    minimumHeight: 400 * dpiScale;
    visible: true;
    visibility: Window.Maximized;
    color: "#ffffff";

    property int columns: 14;
    property int rows: 8;
    property real tileSize: Math.min(root.height / (rows + 2), root.width / (columns + 2));

    title: qsTr("Calibration target") + ` (${columns} x ${rows})`;

    Component.onCompleted: {
        ui_tools.set_icon(root);
        if (Qt.platform.os == "android" || Qt.platform.os == "ios") {
            flags = Qt.WindowStaysOnTopHint;
        }
    }
    Column {
        anchors.centerIn: parent;
        Repeater {
            model: (root.rows + 1);
            Row {
                property int colIndex: index;
                Repeater {
                    model: (root.columns + 1);
                    Rectangle {
                        width: root.tileSize;
                        height: width;
                        color: ((colIndex % 2 == 0)? (index % 2 != 0) : (index % 2 == 0))? "white" : "black";
                        radius: colIndex == 0 || index == 0 || colIndex == rows || index == columns? height : 0;

                        Rectangle {
                            y: colIndex == 0? height : 0;
                            x: index == 0? width : 0;
                            width: parent.width / (index == 0 || index == columns? 2 : 1);
                            height: parent.height / (colIndex == 0 || colIndex == rows? 2 : 1);
                            color: parent.color;
                            visible: parent.radius > 0;
                        }

                        Rectangle {
                            visible: ((colIndex == 4 || (colIndex == 3 && index == 7)) && (index == 7 || index == 8));
                            width: parent.width / 3;
                            height: width;
                            radius: height;
                            anchors.centerIn: parent;
                            color: parent.color == "#000000"? "white" : "black";
                        }
                    }
                }
            }
        }
    }

    WindowCloseButton { onClicked: root.close(); }
}
