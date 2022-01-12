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

    function updateEntry(key, value) {
        model[key] = value;
        modelChanged();
    }
    Column {
        id: col1;
        spacing: tl.spacing;
        property var keys: Object.keys(tl.model);
        Repeater {
            model: col1.keys;
            BasicText { text: qsTr(modelData) + ":"; anchors.right: parent.right; leftPadding: 0; }
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
                BasicText {
                    id: t2;
                    text: modelData;
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
                onAccepted: {
                    visible = false;
                    parent.parent.parent.children[0].visible = true;
                    if (desc.onChange)
                        desc.onChange(newValue.allowText? text : value);
                }
            }
            LinkButton {
                anchors.verticalCenter: parent.verticalCenter;
                icon.name: newValue.visible? "checkmark" : "pencil";
                icon.height: parent.height * 0.8;
                icon.width: parent.height * 0.8;
                height: newValue.visible? newValue.height + 5 * dpiScale : undefined;
                leftPadding: newValue.visible? 15 * dpiScale : 0; rightPadding: leftPadding;
                onClicked: {
                    if (newValue.visible) {
                        newValue.accepted();
                    } else {
                        const val = desc.value();
                        if (typeof val === "string") newValue.text = val;
                        else newValue.value = val;
                        newValue.visible = true;
                        parent.parent.parent.children[0].visible = false;
                    }
                }
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
