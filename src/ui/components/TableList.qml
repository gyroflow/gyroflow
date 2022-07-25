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
    property real spacing: 8 * dpiScale;

    property var editableFields: ({});
    property var editableKeys: Object.keys(editableFields);

    signal commitAll();

    function updateEntry(key: string, value: string) {
        model[key] = value;
        let index = Object.keys(tl.model).indexOf(key);
        if (index !== -1) {
            col2.children[index].children[0].text = value;
        }
    }
    function updateEntryWithTrigger(key: string, value: string) {
        updateEntry(key, value);

        const desc = tl.editableFields[key];
        if (desc && desc.onChange) {
            desc.onChange(value);
        }
    }
    Column {
        id: col1;
        spacing: tl.spacing;
        property var keys: Object.keys(tl.model);
        Repeater {
            model: col1.keys;
            BasicText {
                text: qsTr(modelData) + ":";
                onTextChanged: Qt.callLater(updateHeights);
                anchors.right: parent.right;
                leftPadding: 0;
                objectName: "left";
            }
        }
    }
    Column {
        id: col2;
        spacing: tl.spacing;
        Repeater {
            model: Object.values(tl.model);
            Row {
                height: t2.height;
                spacing: 5 * dpiScale;
                objectName: "right";
                BasicText {
                    id: t2;
                    text: modelData;
                    onTextChanged: Qt.callLater(updateHeights);
                    font.bold: true;
                }
                Loader {
                    height: parent.height;
                    property var name: col1.keys[index];
                    sourceComponent: editableKeys.includes(col1.keys[index])? editable : undefined;
                }
            }
        }
    }
    function updateHeights() {
        for (let i = 0; i < col1.children.length; ++i) {
            if (i < col2.children.length) {
                const l = col1.children[i];
                const r = col2.children[i];
                if (l.objectName == "left" && r.objectName == "right") {
                    r.height = l.height = Math.max(l.height, r.height);
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
                    function onCommitAll() { if (newValue.visible) newValue.accepted(); }
                }
            }
            LinkButton {
                id: editLinkBtn;
                anchors.verticalCenter: parent.verticalCenter;
                icon.name: newValue.visible? "checkmark" : "pencil";
                icon.source: newValue.visible? "qrc:/resources/icons/svg/checkmark.svg" : "qrc:/resources/icons/svg/pencil.svg";
                icon.height: parent.height * 0.8;
                icon.width: parent.height * 0.8;
                opacity: editLinkBtn.activeFocus ? 0.8 : 1;
                height: newValue.visible? newValue.height + 5 * dpiScale : undefined;
                leftPadding: newValue.visible? 15 * dpiScale : 0; rightPadding: leftPadding;

                function _onClicked() {
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
}
