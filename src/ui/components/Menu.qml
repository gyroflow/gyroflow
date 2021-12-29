import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC;
import QtQuick.Controls.Material 2.15 as QQCM
import QtQuick.Controls.Material.impl 2.15 as QQCMI
import QtQuick.Controls.impl 2.15 as QQCI;

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

        property color textColor: orgIconColor.a > 0.1? orgIconColor : styleTextColor;
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
            color: dlg.hovered || dlg.highlighted? styleHighlightColor : "transparent";
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
