// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC

TextField {
    id: root;
    property int precision: 0;
    property string unit: "";
    property real value: 0;
    property bool preventChange: false;
    property alias from: validator.bottom;
    property alias to: validator.top;
    property bool live: true;
    property real defaultValue: NaN;
    property bool allowText: false;
    property bool intNoThousandSep: false;
    property var reset: () => { value = defaultValue; };

    property string keyframe: "";
    property bool keyframesEnabled: false;
    property real finalValue: value;

    property alias contextMenu: menuLoader.sourceComponent;

    onFinalValueChanged: {
        if (keyframe && keyframesEnabled) {
            controller.set_keyframe(keyframe, window.videoArea.timeline.getTimestampUs(), finalValue);
        }
    }

    Keys.onDownPressed: (e) => {
        const lastDigit = Math.pow(10, precision);
        if (allowText) return;
             if (e.modifiers & Qt.AltModifier) value -= 1 / lastDigit;
        else if (e.modifiers & Qt.ControlModifier) value -= 100 / lastDigit;
        else if (e.modifiers & Qt.ShiftModifier) value -= 1000 / lastDigit;
        else value -= 10 / lastDigit;
    }
    Keys.onUpPressed: (e) => {
        const lastDigit = Math.pow(10, precision);
        if (allowText) return;
             if (e.modifiers & Qt.AltModifier) value += 1 / lastDigit;
        else if (e.modifiers & Qt.ControlModifier) value += 100 / lastDigit;
        else if (e.modifiers & Qt.ShiftModifier) value += 1000 / lastDigit;
        else value += 10 / lastDigit;
    }
    onValueChanged: {
        if (preventChange || allowText) return;
        text = intNoThousandSep ? (Math.round(value)).toString() : value.toLocaleString(Qt.locale(), "f", precision);
    }
    function updateValue() {
        if (allowText) return;
        preventChange = true;
        try {
            value = Number.fromLocaleString(Qt.locale(), text.replace(/\s+/g, ""));
        } catch(e) {
            console.error(e, Qt.locale(), text);
        }
        preventChange = false;
    }
    onTextChanged: if (live) updateValue();
    onEditingFinished: updateValue();

    Component.onCompleted: {
        if (isNaN(defaultValue)) defaultValue = value;
        valueChanged();
    }
    onAccepted: valueChanged();
    onFocusChanged: if (!activeFocus) valueChanged();

    Rectangle {
        visible: !root.acceptableInput && !allowText;
        anchors.fill: parent;
        color: "transparent";
        radius: root.background.radius;
        border.color: "#c33838";
        border.width: 1 * dpiScale;
    }

    inputMethodHints: allowText? Qt.ImhNone : (Qt.ImhPreferNumbers | Qt.ImhFormattedNumbersOnly)

    validator: DoubleValidator { id: validator; decimals: root.precision }

    onAllowTextChanged: {
        if (allowText) root.validator = null;
    }

    ContextMenuMouseArea {
        cursorShape: Qt.ibeam;
        underlyingItem: root;
        onContextMenu: (isHold, x, y) =>  menuLoader.popup(root, x, y);
    }

    Component {
        id: defaultMenu;
        Menu {
            font.pixelSize: 11.5 * dpiScale;
            Action {
                iconName: "undo";
                text: qsTr("Reset value");
                enabled: value != defaultValue;
                onTriggered: root.reset()
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
        }
    }
    ContextMenuLoader {
        id: menuLoader;
        sourceComponent: defaultMenu
    }

    BasicText {
        visible: !!root.unit;
        x: parent.contentWidth;
        text: root.unit;
        height: parent.height;
        verticalAlignment: Text.AlignVCenter;
    }
}
