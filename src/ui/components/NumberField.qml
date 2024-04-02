// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC

TextField {
    id: root;
    overwriteMode: false;

    property int precision: 0;
    property string unit: "";
    property real value: 0;
    property bool preventChange: false;
    property alias from: validator.bottom;
    property alias to: validator.top;
    property bool live: true;
    property real defaultValue: NaN;
    property bool allowText: false;
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
        if (allowText) return;
        const locale = Qt.locale();
        const result = ui_tools.modify_digit(root.text.replace(locale.decimalPoint, '.'), root.cursorPosition, false).split(';');
        root.text = result[0].replace('.', locale.decimalPoint);
        root.cursorPosition = result[1];
    }
    Keys.onUpPressed: (e) => {
        if (allowText) return;
        const locale = Qt.locale();
        const result = ui_tools.modify_digit(root.text.replace(locale.decimalPoint, '.'), root.cursorPosition, true).split(';');
        root.text = result[0].replace('.', locale.decimalPoint);
        root.cursorPosition = result[1];
    }
    Keys.onPressed: (e) => {
        if (e.key == Qt.Key_Insert) {
            overwriteMode = !overwriteMode;
        }
    }
    onValueChanged: {
        if (preventChange || allowText) return;
        let locale = Qt.locale();
        locale.numberOptions = Locale.OmitGroupSeparator;
        text = Number(value).toLocaleString(locale, "f", precision);
    }
    function updateValue(): void {
        if (allowText) return;
        preventChange = true;
        let locale = Qt.locale();
        locale.numberOptions = Locale.OmitGroupSeparator;
        try {
            value = Number.fromLocaleString(locale, text.replace(/\s+/g, ""));
        } catch(e) {
            let locale = Qt.locale();
            locale.numberOptions = Locale.OmitGroupSeparator;
            console.error(e, Qt.locale(), text, (11234.56).toLocaleString(Qt.locale(), "f", precision), (11234.56).toLocaleString(locale, "f", precision));
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
