// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls as QQC

// Search changes the popup rows, never the selected value or original indices.
QQC.ComboBox {
    id: root;
    height: 35 * dpiScale;
    font.pixelSize: 13 * dpiScale;
    font.family: styleFont;
    hoverEnabled: enabled;
    property bool includeOther: false;
    property string query: "";
    property var searchAliases: ({});
    property var filteredOptions: Array.from(model || []).map((text, index) => ({text:text, index:index})).filter(item =>
        (includeOther && item.index === count - 1) || query.toLowerCase().trim().split(/\s+/).every(word =>
            [item.text].concat(searchAliases[item.text] || []).some(alias => alias.toLowerCase().replace(/[-_\s]/g, "").indexOf(word.replace(/[-_\s]/g, "")) >= 0)));

    function choose(index: int): void {
        currentIndex = index;
        activated(index);
        picker.close();
    }
    indicator: DropdownChevron { height: root.height / 2.4; }
    background: Rectangle {
        color: root.hovered || root.activeFocus ? Qt.lighter(styleButtonColor, 1.2) : styleButtonColor;
        radius: 6 * dpiScale;
        opacity: root.enabled ? 1 : 0.5;
    }
    contentItem: Text {
        text: root.displayText;
        textFormat: Text.PlainText;
        color: styleTextColor;
        font: root.font;
        verticalAlignment: Text.AlignVCenter;
        leftPadding: 10 * dpiScale;
        rightPadding: 24 * dpiScale;
        elide: Text.ElideRight;
        opacity: root.enabled ? 1 : 0.5;
    }
    Keys.onPressed: event => {
        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space || event.key === Qt.Key_Down) {
            picker.open();
            event.accepted = true;
        }
    }
    popup: QQC.Popup {
        id: picker;
        focus: true;
        property alias lv: list;
        width: Math.max(root.width, 360 * dpiScale);
        implicitHeight: Math.min(280 * dpiScale, (Math.max(1, list.count) * 35 + 42) * dpiScale);
        padding: 4 * dpiScale;
        onOpened: {
            search.text = "";
            list.currentIndex = root.filteredOptions.findIndex(item => item.index === root.currentIndex);
            if (list.currentIndex >= 0) list.positionViewAtIndex(list.currentIndex, ListView.Contain);
            Qt.callLater(() => { if (picker.visible) search.forceActiveFocus(); });
        }
        onClosed: { root.query = ""; root.forceActiveFocus(); }
        background: Rectangle { color: styleButtonColor; border.color: stylePopupBorder; radius: 5 * dpiScale; }
        contentItem: Column {
            spacing: 4 * dpiScale;
            TextField {
                id: search;
                objectName: "catalogueFilter";
                width: parent.width;
                placeholderText: qsTr("Type to filter…");
                onTextChanged: root.query = text;
                onAccepted: if (list.count) root.choose(root.filteredOptions[Math.max(0, list.currentIndex)].index);
                Keys.onDownPressed: { list.currentIndex = 0; list.forceActiveFocus(); }
                Keys.onEscapePressed: picker.close();
            }
            ListView {
                id: list;
                width: parent.width;
                height: picker.availableHeight - search.height - parent.spacing;
                clip: true;
                model: root.filteredOptions;
                keyNavigationEnabled: true;
                onModelChanged: currentIndex = count > 0 ? 0 : -1;
                Keys.onReturnPressed: if (currentIndex >= 0) root.choose(root.filteredOptions[currentIndex].index);
                Keys.onEnterPressed: if (currentIndex >= 0) root.choose(root.filteredOptions[currentIndex].index);
                Keys.onEscapePressed: picker.close();
                QQC.ScrollIndicator.vertical: QQC.ScrollIndicator { }
                delegate: QQC.ItemDelegate {
                    required property var modelData;
                    required property int index;
                    width: list.width;
                    height: 35 * dpiScale;
                    text: modelData.text;
                    font: root.font;
                    highlighted: list.currentIndex === index;
                    contentItem: Text { textFormat: Text.PlainText; text: parent.text; color: styleTextColor; font: root.font; elide: Text.ElideRight; verticalAlignment: Text.AlignVCenter; }
                    background: Rectangle { color: parent.hovered || parent.highlighted ? styleHighlightColor : "transparent"; radius: 4 * dpiScale; }
                    onClicked: root.choose(modelData.index);
                }
                Text {
                    anchors.centerIn: parent;
                    visible: list.count === 0;
                    text: qsTr("No matches");
                    color: styleTextColor;
                    font: root.font;
                }
            }
        }
    }
}
