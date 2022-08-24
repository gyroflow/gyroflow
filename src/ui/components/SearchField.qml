// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC
import QtQuick.Controls.impl as QQCI

TextField {
    id: root;
    property var model: [];
    property alias popup: popup;
    property var profilesMenu: null;

    signal selected(var item);

    Popup {
        id: popup;
        model: root.model;
        y: parent.height + 2 * dpiScale;
        font.pixelSize: 12 * dpiScale;
        itemHeight: 25 * dpiScale;
        width: Math.max(parent.width * 1.5, Math.min(window.width * 0.8, maxItemWidth + 10 * dpiScale));
        onClicked: (index) => {
            root.selected(model[index]);
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

        let i = 0;
        for (const x of root.model) {
            const test = x[0].toLowerCase();
            let add = true;
            for (const word of words) {
                if (test.indexOf(word) < 0) {
                    add = false;
                    break;
                }
            }

            if (add) {
                m.push(x);
            }

            ++i;
        }

        if (!m.length) popup.close();
        else {
            m.sort((a, b) => {
                const aPriority = a[1].endsWith(".gyroflow") || root.profilesMenu.favorites[a[2]];
                const bPriority = b[1].endsWith(".gyroflow") || root.profilesMenu.favorites[b[2]];
                if (aPriority && bPriority) return a[0].localeCompare(b[0]);
                if (aPriority) return -1;
                if (bPriority) return 1;
                return a[0].localeCompare(b[0]);
            });
        }

        popup.maxItemWidth = 0;
        popup.model = [];
        popup.model = m;
        popup.currentIndex = -1;
    }
    Keys.onDownPressed: {
        if (!popup.opened) {
            popup.open();
        } else {
            popup.highlightedIndex = Math.min(popup.model.length - 1, popup.highlightedIndex + 1);
        }
    }
    Keys.onUpPressed: popup.highlightedIndex = Math.max(0, popup.highlightedIndex - 1);
    onAccepted: {
        if (popup.opened) {
            root.selected(popup.model[popup.highlightedIndex]);
            popup.close();
            root.text = "";
        }
    }
}
