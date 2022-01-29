// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick

Rectangle {
    property QtObject timeline: null;
    property int org_timestamp_us: 0;
    property real position: 0;
    property real value: 0;
    property string unit: "";

    property bool isCalibPoint: false;

    id: root;

    x: timeline.mapToVisibleArea(position) * (parent.width);
    y: 35 * dpiScale;
    radius: width;
    height: parent.height - 25 * dpiScale;
    width: 1 * dpiScale;
    color: "#dcae24";
    visible: x >= 0 && x <= parent.width;

    signal edit(real timestamp_us, real val);
    signal remove(real timestamp_us);
    signal zoomIn(real timestamp_us);

    Rectangle {
        height: 12 * dpiScale;
        width: 13 * dpiScale;
        color: root.color;
        radius: 3 * dpiScale;
        //y: -5 * dpiScale;
        x: -width / 2;
        anchors.bottom: parent.bottom;
        opacity: ma.containsMouse? 0.8 : 1.0;

        Rectangle {
            height: 11 * dpiScale;
            width: 11 * dpiScale;
            color: parent.color;
            radius: 3 * dpiScale;
            anchors.centerIn: parent;

            anchors.verticalCenterOffset: -3 * dpiScale;
            rotation: 45;
        }
        MouseArea {
            id: ma;
            hoverEnabled: true;
            anchors.fill: parent;
            anchors.margins: -15 * dpiScale;
            cursorShape: Qt.PointingHandCursor;
            onClicked: (mouse) => {
                if (mouse.button === Qt.LeftButton) {
                    root.edit(root.org_timestamp_us, root.value);
                } else {
                    menu.popup();
                }
            }
            onPressAndHold: (mouse) => {
                if ((Qt.platform.os == "android" || Qt.platform.os == "ios") && mouse.button !== Qt.RightButton) {
                    menu.popup()
                } else {
                    mouse.accepted = false;
                }
            }
            onDoubleClicked: root.zoomIn(root.org_timestamp_us + root.value * 1000.0); 
            acceptedButtons: Qt.LeftButton | Qt.RightButton
        }
        BasicText {
            id: t;
            leftPadding: 0;
            text: root.value.toFixed(2) + " " + root.unit;
            anchors.horizontalCenter: parent.horizontalCenter;
            y: 16 * dpiScale;
            font.pixelSize: 11 * dpiScale;
        }

        Menu {
            id: menu;
            Action {
                id: editAction;
                text: qsTr("Edit offset");
                icon.name: "pencil";
                onTriggered: root.edit(root.org_timestamp_us, root.value);
            }
            Action {
                text: isCalibPoint? qsTr("Delete calibration point") : qsTr("Delete sync point");
                icon.name: "bin;#f67575";
                onTriggered: root.remove(root.org_timestamp_us);
            }
            Action {
                id: zoomAction;
                text: qsTr("Zoom in");
                icon.name: "search";
                onTriggered: root.zoomIn(root.org_timestamp_us + root.value * 1000.0);
            }
            Component.onCompleted: {
                if (isCalibPoint) {
                    menu.removeAction(editAction);
                    menu.removeAction(zoomAction);
                }
            }
        }
    }
}
