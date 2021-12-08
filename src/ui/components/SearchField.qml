import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC
import QtQuick.Controls.impl 2.15 as QQCI

TextField {
    id: root;
    property var model: [];

    signal selected(var text, int index);

    Popup {
        id: popup;
        model: root.model;
        width: parent.width * 1.1;
        y: parent.height + 2 * dpiScale;
        font.pixelSize: 12 * dpiScale;
        itemHeight: 25 * dpiScale;
        property var indexMapping: [];
        onClicked: (index) => {
            root.selected(model[index], indexMapping[index]);
            popup.close();
            root.text = "";
        }
    }

    rightPadding: 30 * dpiScale;
    QQCI.IconImage {
        name: "search";
        color: styleTextColor;
        anchors.right: parent.right
        anchors.rightMargin: 5 * dpiScale;
        height: Math.round(parent.height * 0.7)
        width: height;
        layer.enabled: true;
        layer.textureSize: Qt.size(height*2, height*2);
        layer.smooth: true;
        anchors.verticalCenter: parent.verticalCenter;
    }

    onTextChanged: {
        if (!text) return popup.close();
        if (!popup.opened) popup.open();
        const s = text.toLowerCase();
        let m = [];
        let indexMapping = [];

        let i = 0;
        for (const x of root.model) {
            if (x.toLowerCase().indexOf(s) > -1) {
                m.push(x);
                indexMapping.push(i);
            }
            ++i;
        }
        if (!m.length) popup.close();
        popup.model = m;
        popup.indexMapping = indexMapping;
        popup.currentIndex = -1;
        // Trigger reposition
        popup.topMargin = 1;
        popup.topMargin = 0;
    }
    Keys.onDownPressed: popup.highlightedIndex = Math.min(popup.model.length - 1, popup.highlightedIndex + 1);
    Keys.onUpPressed: popup.highlightedIndex = Math.max(0, popup.highlightedIndex - 1);
    onAccepted: {
        if (popup.opened) {
            root.selected(popup.model[popup.highlightedIndex], popup.indexMapping[popup.highlightedIndex]);
            popup.close();
            root.text = "";
        }
    }
}
