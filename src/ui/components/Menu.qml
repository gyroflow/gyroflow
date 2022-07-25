// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC;
import QtQuick.Controls.Material as QQCM
import QtQuick.Controls.Material.impl as QQCMI
import QtQuick.Controls.impl as QQCI;

QQC.Menu {
    id: menu;
    property real maxItemWidth: 50 * dpiScale;
    property real itemHeight: 27 * dpiScale;
    verticalPadding: 3 * dpiScale;
    leftPadding: 4 * dpiScale;
    rightPadding: 4 * dpiScale;
    font.pixelSize: 11.5 * dpiScale;

    property var colors: [];

    delegate: QQC.MenuItem {
        id: dlg;

        property color textColor: orgIconColor.a > 0.1? orgIconColor : (dlg.checked? styleTextColorOnAccent : styleTextColor);
        QQCM.Material.foreground: textColor;

        property color orgIconColor: "transparent";

        leftPadding: 8 * dpiScale;
        rightPadding: 8 * dpiScale;
        topPadding: 5 * dpiScale;
        bottomPadding: 5 * dpiScale;
        spacing: 8 * dpiScale;
        icon.width: menu? (menu.itemHeight / 2 + 1 * dpiScale) : 0;
        icon.height: menu? (menu.itemHeight / 2 + 1 * dpiScale) : 0;
        font: menu? menu.font : undefined;

        Component.onCompleted: {
            if (icon.name && icon.name.indexOf(";") > 0) {
                const parts = icon.name.split(";");
                dlg.orgIconColor = parts[1];
                icon.name = parts[0];
                icon.source = "qrc:/resources/icons/svg/" + parts[0] + ".svg";
            }
            Qt.callLater(function() {
                if (menu && dlg && dlg.implicitContentWidth > menu.maxItemWidth) menu.maxItemWidth = dlg.implicitContentWidth;
            });
        }

        indicator: Item {
            height: parent.height;
            width: -dlg.spacing / 2;
            Rectangle {
                x: 1 * dpiScale;
                color: styleAccentColor;
                height: parent.height * 0.6;
                width: 3 * dpiScale;
                radius: width;
                y: (parent.height - height) / 2;
                visible: dlg.checked && dlg.checkable;
            }
        }
        scale: dlg.down? 0.970 : 1.0;
        Ease on scale { }

        background: Rectangle {
            color: dlg.checked? styleAccentColor : (dlg.hovered || dlg.highlighted? styleHighlightColor : "transparent");
            implicitHeight: itemHeight;
            radius: 4 * dpiScale;
        }
    }

    background: Rectangle {
        color: styleButtonColor;
        border.width: 1 * dpiScale;
        border.color: stylePopupBorder
        implicitWidth: menu.maxItemWidth + 40 * dpiScale;
        radius: 4 * dpiScale;
        layer.enabled: true
        layer.effect: QQCMI.ElevationEffect { elevation: 8 }
    }
}
