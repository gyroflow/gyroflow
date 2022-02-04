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

    Keys.onDownPressed: (e) => {
        if (allowText) return;
             if (e.modifiers & Qt.AltModifier) value -= 0.001;
        else if (e.modifiers & Qt.ControlModifier) value -= 0.1;
        else if (e.modifiers & Qt.ShiftModifier) value -= 1;
        else value -= 0.01;
    }
    Keys.onUpPressed: (e) => {
        if (allowText) return;
             if (e.modifiers & Qt.AltModifier) value += 0.001;
        else if (e.modifiers & Qt.ControlModifier) value += 0.1;
        else if (e.modifiers & Qt.ShiftModifier) value += 1;
        else value += 0.01;
    }
    Keys.onPressed: (e) => {
        if (!allowText && e.key == Qt.Key_Space) {
            root.focus = false;
            window.videoArea.timeline.focus = true;
            const vid = window.videoArea.vid;
            if (vid.playing) vid.pause(); else vid.play();
            e.accepted = true;
        }
    }

    onValueChanged: {
        if (preventChange || allowText) return;
        text = value.toLocaleString(Qt.locale(), "f", precision);
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

    BasicText {
        visible: !!root.unit;
        x: parent.contentWidth;
        text: root.unit;
        height: parent.height;
        verticalAlignment: Text.AlignVCenter;
    }
}
