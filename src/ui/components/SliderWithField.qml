// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC

Row {
    id: root;
    spacing: 5 * dpiScale;
    height: 25 * dpiScale;
    property alias slider: slider;
    property alias field: field;
    property alias defaultValue: field.defaultValue;
    property alias from: slider.from;
    property alias to: slider.to;
    property alias live: slider.live;
    property alias unit: field.unit;
    property alias precision: field.precision;
    property string keyframe: "";
    property bool keyframesEnabled: false;
    property real scaler: 1;

    property bool preventChange: false;

    property real value: defaultValue;

    property alias contextMenu: menuLoader.sourceComponent;

    onValueChanged: {
        if (!root.preventChange) {
            field.value = value * scaler;
        }
    }
    Connections {
        target: controller;
        enabled: root.keyframe.length > 0;
        function onKeyframe_value_updated(keyframe: string, value: real) {
            if (keyframe == root.keyframe) {
                root.preventChange = true;
                field.value = value * root.scaler;
                root.preventChange = false;
            }
        }
    }

    Slider {
        id: slider;
        width: parent.width - field.width - root.spacing;
        anchors.verticalCenter: parent.verticalCenter;
        property bool preventChange: false;
        onValueChanged: if (!preventChange) field.value = value;
        unit: field.unit;
        precision: field.precision;

        ContextMenuMouseArea {
            underlyingItem: slider;
            onContextMenu: (isHold, x, y) => menuLoader.popup(slider, x, y);
        }

        Component {
            id: defaultMenu;
            Menu {
                font.pixelSize: 11.5 * dpiScale;
                Action {
                    iconName: "undo";
                    text: qsTr("Reset value");
                    enabled: field.value != defaultValue;
                    onTriggered: {
                        field.value = defaultValue;
                    }
                }
                Action {
                    iconName: "keyframe";
                    enabled: root.keyframe.length > 0;
                    text: qsTr("Enable keyframing");
                    checked: root.keyframesEnabled;
                    onTriggered: {
                        checked = !checked;
                        root.keyframesEnabled = checked;
                        if (!checked) {
                            controller.clear_keyframes_type(root.keyframe);
                        }
                    }
                }
                Action {
                    iconName: "plus";
                    enabled: root.keyframe.length > 0 && root.keyframesEnabled;
                    text: qsTr("Add keyframe");
                    onTriggered: controller.set_keyframe(root.keyframe, window.videoArea.timeline.getTimestampUs(), root.value);
                }
            }
        }
        ContextMenuLoader {
            id: menuLoader;
            sourceComponent: defaultMenu
        }
    }
    NumberField {
        id: field;
        width: 50 * dpiScale;
        height: 25 * dpiScale;
        precision: 3;
        font.pixelSize: 11 * dpiScale;
        anchors.verticalCenter: parent.verticalCenter;
        contextMenu: root.contextMenu;
        onValueChanged: {
            slider.preventChange = true;
            slider.value = value;
            Qt.callLater(() => { if (slider) slider.preventChange = false; });

            if (!root.preventChange) {
                root.preventChange = true;
                root.value = value / root.scaler;
                root.preventChange = false;

                if (root.keyframe && root.keyframesEnabled) {
                    controller.set_keyframe(root.keyframe, window.videoArea.timeline.getTimestampUs(), root.value);
                }
            }
        }
    }
}
