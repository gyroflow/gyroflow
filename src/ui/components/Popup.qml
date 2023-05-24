// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC
import QtQuick.Controls.impl as QQCI
import QtQuick.Controls.Material.impl as QQCMI

QQC.Popup {
    id: popup;
    width: parent.width;
    implicitHeight: (lv.count * itemHeight) + 4 * dpiScale;
    padding: 2 * dpiScale;
    property alias model: lv.model;
    property alias currentIndex: lv.currentIndex;
    property alias lv: lv;
    property int highlightedIndex: currentIndex;
    property real itemHeight: 35 * dpiScale;
    font.pixelSize: 12 * dpiScale;
    font.family: styleFont;
    property real maxItemWidth: 0;

    property var icons: [];
    property var colors: [];

    signal clicked(int index);

    contentItem: ListView {
        id: lv;
        clip: true;
        QQC.ScrollIndicator.vertical: QQC.ScrollIndicator { }
        delegate: PopupDelegate { parentPopup: popup; lv: popup.lv; }
        highlightFollowsCurrentItem: true;
        focus: true;
        keyNavigationEnabled: true;
        highlight: Rectangle {
            color: styleHighlightColor;
            radius: 4 * dpiScale;
        }
    }

    background: Rectangle {
        color: styleButtonColor;
        border.width: 1 * dpiScale;
        border.color: stylePopupBorder
        radius: 4 * dpiScale;
        layer.enabled: true;
        layer.effect: QQCMI.ElevationEffect { elevation: 8 }
    }
}
