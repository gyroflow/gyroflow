// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC
import QtQuick.Controls.impl as QQCI

TextField {
    id: root;
    property var model: [];
    property alias popup: popup;

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
        const searchTerm = text.toLowerCase();
        const words = searchTerm.split(/[\s,;]+/).filter(s => s);
        
        if (!words.length) {
            popup.close();
            return;
        }
        if (!popup.opened) popup.open();

        let m = [];
        let indexMapping = [];

        let i = 0;
        for (const x of root.model) {
            const test = x.toLowerCase();
            let add = true;
            for (const word of words) {
                if (test.indexOf(word) < 0) {
                    add = false;
                    break;
                }
            }
            
            if (add) {
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
