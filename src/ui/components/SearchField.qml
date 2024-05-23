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

    Component.onCompleted: root.children[0].x = root.leftPadding; // Set x of PlaceholderText

    Popup {
        id: popup;
        model: [];
        y: parent.height + 2 * dpiScale;
        font.pixelSize: 12 * dpiScale;
        itemHeight: 25 * dpiScale;
        width: window.isMobileLayout? parent.width : Math.max(parent.width * 1.5, Math.min(window.width * 0.8, maxItemWidth + 10 * dpiScale));
        onClicked: (index) => {
            root.selected(model[index]);
            popup.close();
            root.text = "";
        }
    }

    rightPadding: 30 * dpiScale;
    QQCI.IconImage {
        name: "search";
        source: "qrc:/resources/icons/svg/search.svg";
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

    Connections {
        target: controller;
        function onSearch_lens_profile_finished(profiles: list<var>): void {
            if (!popup.opened && profiles.length > 0) popup.open();
            if (popup.opened && !profiles.length) popup.close();
            popup.maxItemWidth = 0;
            popup.model = [];
            popup.model = profiles;
            popup.currentIndex = -1;
        }
    }

    onTextChanged: {
        controller.search_lens_profile(text, Object.keys(root.profilesMenu.favorites), profilesMenu.currentVideoAspectRatio, profilesMenu.currentVideoAspectRatioSwapped);
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
