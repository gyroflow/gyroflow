// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as QQC

ComboBox {
    id: control;
    property string fieldLabel: "";
    Accessible.name: fieldLabel + ": " + displayText;
    tooltip: displayText;

    Keys.onPressed: (event) => {
        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
            choices.open();
            event.accepted = true;
        }
    }

    Keys.onReleased: (event) => {
        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) event.accepted = true;
    }

    function choose(index: int): void {
        if (index < 0 || index >= (model || []).length) return;
        currentIndex = index;
        activated(index);
        popup.close();
        forceActiveFocus();
    }

    popup: QQC.Popup {
        id: choices;
        focus: true;
        property real maxItemWidth: control.width;
        property real itemHeight: 35 * dpiScale;
        property var filteredIndices: {
            const needle = filter.text.trim().toLowerCase();
            const values = control.model || [];
            let result = [];
            for (let i = 0; i < values.length; ++i) {
                if (!needle || values[i].toLowerCase().indexOf(needle) >= 0) result.push(i);
            }
            return result;
        }
        width: control.width;
        implicitHeight: filter.height + Math.max(1, Math.min(8, filteredIndices.length)) * itemHeight + 12 * dpiScale;
        padding: 4 * dpiScale;
        margins: 8 * dpiScale;
        closePolicy: QQC.Popup.CloseOnEscape | QQC.Popup.CloseOnPressOutsideParent;
        onOpened: {
            filter.text = "";
            options.currentIndex = Math.max(0, filteredIndices.indexOf(control.currentIndex));
            options.positionViewAtIndex(options.currentIndex, ListView.Contain);
            filter.forceActiveFocus();
        }
        contentItem: Column {
            spacing: 4 * dpiScale;
            TextField {
                id: filter;
                objectName: "cameraSelectorFilter";
                width: parent.width;
                // Keep the hint inside the compact field on every Qt Controls style.
                placeholderText: "";
                BasicText {
                    anchors.fill: parent;
                    anchors.leftMargin: filter.leftPadding;
                    anchors.rightMargin: filter.rightPadding;
                    text: qsTr("Search...");
                    color: filter.placeholderTextColor;
                    verticalAlignment: Text.AlignVCenter;
                    elide: Text.ElideRight;
                    visible: filter.text.length === 0;
                }
                Accessible.name: qsTr("Search %1").arg(control.fieldLabel);
                onTextChanged: options.currentIndex = 0;
                Keys.onReturnPressed: control.choose(choices.filteredIndices[options.currentIndex] ?? -1);
                Keys.onEnterPressed: control.choose(choices.filteredIndices[options.currentIndex] ?? -1);
                Keys.onDownPressed: options.currentIndex = Math.min(options.count - 1, options.currentIndex + 1);
                Keys.onUpPressed: options.currentIndex = Math.max(0, options.currentIndex - 1);
            }
            ListView {
                id: options;
                objectName: "cameraSelectorOptions";
                width: parent.width;
                height: parent.height - filter.height - parent.spacing;
                clip: true;
                model: choices.filteredIndices;
                onCountChanged: currentIndex = count > 0 ? 0 : -1;
                keyNavigationEnabled: true;
                QQC.ScrollIndicator.vertical: QQC.ScrollIndicator { }
                delegate: QQC.ItemDelegate {
                    id: choice;
                    required property int modelData;
                    required property int index;
                    width: options.width;
                    height: choices.itemHeight;
                    text: qsTranslate("Popup", control.model[modelData]);
                    Accessible.name: text;
                    contentItem: BasicText {
                        text: choice.text;
                        verticalAlignment: Text.AlignVCenter;
                        elide: Text.ElideRight;
                    }
                    background: Rectangle {
                        color: choice.hovered || options.currentIndex === choice.index? styleHighlightColor : "transparent";
                        radius: 4 * dpiScale;
                    }
                    onClicked: control.choose(modelData);
                }
                Keys.onReturnPressed: control.choose(choices.filteredIndices[currentIndex] ?? -1);
                BasicText {
                    visible: options.count === 0;
                    anchors.centerIn: parent;
                    text: qsTr("No matches");
                }
            }
        }
        background: Rectangle {
            color: styleButtonColor;
            border.color: stylePopupBorder;
            border.width: 1 * dpiScale;
            radius: 4 * dpiScale;
        }
    }
}
