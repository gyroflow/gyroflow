// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026

import QtQuick

Item {
    id: root;

    property bool hasQuaternions: false;
    property string detectedFormat: "";
    property int integrationMethod: -1;
    property bool applying: false;
    property bool recomputeStarted: false;
    readonly property bool available: hasQuaternions && detectedFormat.toUpperCase().indexOf("DJI") >= 0 && integrationMethod === 0;

    height: content.height;
    onAvailableChanged: if (!available) {
        applying = false;
        recomputeStarted = false;
    }

    Connections {
        target: controller;
        function onCompute_progress(id: real, progress: real): void {
            if (!root.applying)
                return;
            if (progress < 1.0) {
                root.recomputeStarted = true;
            } else if (root.recomputeStarted) {
                root.applying = false;
                root.recomputeStarted = false;
            }
        }
    }

    Column {
        id: content;
        width: parent.width;
        spacing: 4 * dpiScale;

        BasicText {
            text: qsTr("DJI Orientation Repair");
            font.bold: true;
            font.pixelSize: 13 * dpiScale;
        }
        Item {
            width: 1;
            height: 4 * dpiScale;
        }
        Row {
            visible: root.available && window.videoArea.timeline.hasQuaternionSelection;
            spacing: 4 * dpiScale;

            BasicText {
                text: qsTr("Selected:");
                anchors.verticalCenter: parent.verticalCenter;
            }
            Rectangle {
                width: selectedRange.implicitWidth + 8 * dpiScale;
                height: selectedRange.implicitHeight + 4 * dpiScale;
                radius: 3 * dpiScale;
                color: "#f6a10c";

                BasicText {
                    id: selectedRange;
                    anchors.centerIn: parent;
                    leftPadding: 0;
                    color: "#151515";
                    text: qsTr("%1–%2 s").arg((window.videoArea.timeline.quaternionSelectionLeft() * controller.get_scaled_duration_ms() / 1000).toFixed(3)).arg((window.videoArea.timeline.quaternionSelectionRight() * controller.get_scaled_duration_ms() / 1000).toFixed(3));
                }
            }
        }
        BasicText {
            visible: !root.available || !window.videoArea.timeline.hasQuaternionSelection;
            text: root.available
                ? qsTr("Shift-drag the orientation graph to select a range")
                : qsTr("Open a DJI video with orientation metadata to enable repair");
            wrapMode: Text.WordWrap;
            width: parent.width;
            color: styleTextColor;
            opacity: 0.65;
        }
        Row {
            width: parent.width;
            spacing: 4 * dpiScale;
            ComboBox {
                id: strength;
                translateItems: false;
                model: ["Light", "Recommended", "Strong"];
                currentIndex: 1;
                enabled: root.available && !root.applying;
                width: parent.width - smoothButton.width - 4 * dpiScale;
            }
            Button {
                id: smoothButton;
                text: root.applying ? "Waiting" : "Apply";
                enabled: root.available && window.videoArea.timeline.hasQuaternionSelection && !root.applying;
                onClicked: {
                    const timeline = window.videoArea.timeline;
                    const start = timeline.quaternionSelectionLeft() * controller.get_scaled_duration_ms();
                    const end = timeline.quaternionSelectionRight() * controller.get_scaled_duration_ms();
                    root.applying = true;
                    root.recomputeStarted = false;
                    if (controller.smooth_quaternion_range(start, end, strength.currentIndex + 1) > 0) {
                        timeline.clearQuaternionSelection();
                    } else {
                        root.applying = false;
                    }
                }
            }
        }
        Row {
            spacing: 4 * dpiScale;
            Button {
                text: qsTr("Undo");
                enabled: root.available && !root.applying;
                onClicked: controller.undo_quaternion_edit();
            }
            Button {
                text: qsTr("Redo");
                enabled: root.available && !root.applying;
                onClicked: controller.redo_quaternion_edit();
            }
            Button {
                text: "Clear";
                enabled: root.available && !root.applying;
                onClicked: controller.clear_quaternion_edits();
            }
        }
    }
}
