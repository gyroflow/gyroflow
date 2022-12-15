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
        model: [];
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
        const searchTerm = text.toLowerCase()
            .replace("bmpcc4k",  "blackmagic pocket cinema camera 4k")
            .replace("bmpcc6k",  "blackmagic pocket cinema camera 6k")
            .replace("bmpcc",    "blackmagic pocket cinema camera")
            .replace("gopro5",   "hero5 black")  .replace("gopro 5",   "hero5 black")
            .replace("gopro6",   "hero6 black")  .replace("gopro 6",   "hero6 black")
            .replace("gopro7",   "hero7 black")  .replace("gopro 7",   "hero7 black")
            .replace("gopro8",   "hero8 black")  .replace("gopro 8",   "hero8 black")
            .replace("gopro9",   "hero9 black")  .replace("gopro 9",   "hero9 black")
            .replace("gopro10",  "hero10 black") .replace("gopro 10",  "hero10 black")
            .replace("gopro11",  "hero11 black") .replace("gopro 11",  "hero11 black")
            .replace("gopro12",  "hero12 black") .replace("gopro 12",  "hero12 black")
            .replace("session5", "hero5 session").replace("session 5", "hero5 session")
            .replace("a73",      "a7iii")
            .replace("a74",      "a7iv")
            .replace("a7r3",     "a7riii")
            .replace("a7r4",     "a7riv")
            .replace("a7s2",     "a7sii")
            .replace("a7s3",     "a7siii");

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
                // Is preset or favorited
                const aPriority = a[1].endsWith(".gyroflow") || root.profilesMenu.favorites[a[2]];
                const bPriority = b[1].endsWith(".gyroflow") || root.profilesMenu.favorites[b[2]];
                if (aPriority && !bPriority) return -1;
                if (bPriority && !aPriority) return 1;

                // Check aspect match
                const aPriority2 = a[5] != 0 && profilesMenu.currentVideoAspectRatio == a[5];
                const bPriority2 = b[5] != 0 && profilesMenu.currentVideoAspectRatio == b[5];
                if (aPriority2 && !bPriority2) return -1;
                if (bPriority2 && !aPriority2) return 1;

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
            if (!popup.model.length) popup.model = root.model;
            popup.open();
        } else {
            popup.highlightedIndex = Math.min(popup.model.length - 1, popup.highlightedIndex + 1);
        }
    }
    Keys.onEscapePressed: popup.close();
    Keys.onUpPressed: popup.highlightedIndex = Math.max(0, popup.highlightedIndex - 1);
    onAccepted: {
        if (popup.opened) {
            root.selected(popup.model[popup.highlightedIndex]);
            popup.close();
            root.text = "";
        }
    }
}
