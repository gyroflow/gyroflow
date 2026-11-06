// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC

Row {
    id: tl;
    property var model: ({});
    property alias col1: col1;
    property alias col2: col2;
    width: parent.width;
    property real columnSpacing: 8 * dpiScale;
    property bool copyable: false;

    property var editableFields: ({});
    property var editableKeys: Object.keys(editableFields);

    signal commitAll();

    function updateEntry(key: string, value: string): void {
        model[key] = value;
        let index = Object.keys(tl.model).indexOf(key);
        if (index !== -1) {
            col2.children[index].children[0].text = value;
        }
    }
    function updateEntryWithTrigger(key: string, value: string): void {
        updateEntry(key, value);

        const desc = tl.editableFields[key];
        if (desc && desc.onChange) {
            desc.onChange(value);
        }
    }
    Column {
        id: col1;
        spacing: tl.columnSpacing;
        property var keys: Object.keys(tl.model);
        Repeater {
            model: col1.keys;
            BasicText {
                text: modelData? qsTr(modelData) + ":" : " ";
                onTextChanged: Qt.callLater(updateHeights);
                anchors.right: parent.right;
                leftPadding: 0;
                objectName: "left";
            }
        }
    }
    Column {
        id: col2;
        spacing: tl.columnSpacing;
        Repeater {
            model: Object.values(tl.model);
            Row {
                height: t2.height;
                spacing: 5 * dpiScale;
                objectName: "right";
                BasicText {
                    id: t2;
                    text: modelData;
                    visible: (tl.editableFields[col1.keys[index]] || {}).type !== "choice";
                    onTextChanged: Qt.callLater(updateHeights);
                    font.bold: true;
                    MouseArea {
                        enabled: tl.copyable;
                        anchors.fill: parent;
                        acceptedButtons: Qt.LeftButton;
                        onDoubleClicked: controller.copy_to_clipboard(modelData);
                    }
                }
                Loader {
                    height: parent.height;
                    property var name: col1.keys[index];
                    property Item valueText: t2;
                    sourceComponent: editableKeys.includes(name)? (tl.editableFields[name].type === "choice"? choice : editable) : undefined;
                }
            }
        }
    }
    function updateHeights(): void {
        for (let i = 0; i < col1.children.length; ++i) {
            if (i < col2.children.length) {
                const l = col1.children[i];
                const r = col2.children[i];
                if (l.objectName == "left" && r.objectName == "right") {
                    r.height = l.height = Math.max(l.height, r.height);
                    if (l.text == " " && r.children[0].text == " ") {
                        r.height = l.height = 2 * dpiScale;
                    }
                }
            }
        }
    }

    Component {
        id: editable;
        Row {
            id: editableItm;
            spacing: 5 * dpiScale;
            Item { width: 1; height: 1; visible: newValue.visible; }
            property var desc: tl.editableFields[name];
            NumberField {
                id: newValue;
                visible: false;
                x: 5 * dpiScale;
                anchors.verticalCenter: parent.verticalCenter;
                height: parent.height + 8 * dpiScale;
                topPadding: 0; bottomPadding: 0;
                width: (desc.width || 50) * dpiScale;
                font.pixelSize: 12 * dpiScale;
                keyframe: desc.keyframe || "";
                onAccepted: {
                    visible = false;
                    parent.parent.parent.children[0].visible = true;
                    if (desc.onChange)
                        desc.onChange(newValue.allowText? text.trim() : value);
                }
                Connections {
                    target: tl;
                    function onCommitAll(): void { if (newValue.visible) newValue.accepted(); }
                }
            }
            LinkButton {
                id: editLinkBtn;
                anchors.verticalCenter: parent.verticalCenter;
                iconName: newValue.visible? "checkmark" : "pencil";
                icon.height: parent.height * 0.8;
                icon.width: parent.height * 0.8;
                opacity: editLinkBtn.activeFocus ? 0.8 : 1;
                height: newValue.visible? newValue.height + 5 * dpiScale : undefined;
                leftPadding: newValue.visible? 15 * dpiScale : 0; rightPadding: leftPadding;

                function _onClicked(): void {
                    if (newValue.visible) {
                        newValue.accepted();
                    } else {
                        const val = desc.value();
                        if (typeof val === "string") newValue.text = val;
                        else newValue.value = val;
                        newValue.visible = true;
                        parent.parent.parent.children[0].visible = false;
                        newValue.focus = true;
                    }
                }

                onClicked: _onClicked();
                Keys.onReturnPressed: _onClicked();
                Keys.onEnterPressed: _onClicked();

            }
            Component.onCompleted: {
                if (desc.hasOwnProperty("from"))      newValue.from      = desc.from;
                if (desc.hasOwnProperty("to"))        newValue.to        = desc.to;
                if (desc.hasOwnProperty("unit"))      newValue.unit      = desc.unit;
                if (desc.hasOwnProperty("precision")) newValue.precision = desc.precision;
                if (desc["type"] == "text")           newValue.allowText = true;
            }
        }
    }

    // The value is a link that opens a list of `options`, `onChange` gets the picked index
    Component {
        id: choice;
        Item {
            property var desc: tl.editableFields[name];
            width: choiceBtn.x + choiceBtn.width;
            LinkButton {
                id: choiceBtn;
                // Text starts where the value text would
                x: valueText.leftPadding - leftPadding;
                anchors.verticalCenter: parent.verticalCenter;
                leftPadding: 6 * dpiScale;
                rightPadding: 6 * dpiScale;
                text: valueText.text;
                font.bold: true;
                tooltip: (desc.tooltips || [])[desc.value()] || "";
                onClicked: {
                    if (choicePopup.visible) {
                        choicePopup.close();
                    } else {
                        choicePopup.currentIndex = desc.value();
                        choicePopup.open();
                    }
                }
                // Like a value's text: the first click opened the list, the second press closed it
                onDoubleClicked: if (tl.copyable) controller.copy_to_clipboard(valueText.text);
                Popup {
                    id: choicePopup;
                    model: desc.options;
                    width: maxItemWidth + 10 * dpiScale;
                    y: parent.height;
                    itemHeight: 25 * dpiScale;
                    font.pixelSize: 11 * dpiScale;
                    onClicked: i => desc.onChange(i);
                }
            }
        }
    }
}
