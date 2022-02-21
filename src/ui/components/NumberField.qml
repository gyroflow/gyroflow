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
    Keys.onPressed: (e) => {
        if (!allowText && e.key == Qt.Key_Space) {
            root.focus = false;
            window.togglePlay();
            e.accepted = true;
        }
    }
    onValueChanged: {
        if (preventChange || allowText) return;
        text = intNoThousandSep ? (Math.round(value)).toString() : value.toLocaleString(Qt.locale(), "f", precision);
    }
    function updateValue() {
        if (allowText) return;
        preventChange = true;
        value = Number.fromLocaleString(Qt.locale(), text);
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

    MouseArea {
        id: ma;
        anchors.fill: parent;
        acceptedButtons: Qt.LeftButton | Qt.RightButton;
        propagateComposedEvents: true;
        preventStealing: true;
        cursorShape: Qt.ibeam;

        onPressAndHold: (mouse) => {
            if ((Qt.platform.os == "android" || Qt.platform.os == "ios") && mouse.button !== Qt.RightButton) {
                contextMenu.popup();
                mouse.accepted = true;
            } else {
                mouse.accepted = false;
            }
        }

        function _onClicked(mouse) {
            if (mouse.button === Qt.RightButton) {
                contextMenu.popup();
                mouse.accepted = true;
            } else {
                mouse.accepted = false;
            }
        }

        onClicked: (mouse) => _onClicked(mouse);
        onPressed: (mouse) => _onClicked(mouse);
    }

    Menu {
        id: contextMenu;
        font.pixelSize: 11.5 * dpiScale;
        Action {
            icon.name: "undo";
            text: qsTr("Reset value");
            enabled: value != defaultValue;
            onTriggered: {
                value = defaultValue;
            }
        }
    }

    BasicText {
        visible: !!root.unit;
        x: parent.contentWidth;
        text: root.unit;
        height: parent.height;
        verticalAlignment: Text.AlignVCenter;
    }
}
