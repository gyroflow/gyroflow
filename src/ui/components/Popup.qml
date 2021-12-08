import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC
import QtQuick.Controls.impl 2.15 as QQCI
import QtQuick.Controls.Material.impl 2.15 as QQCMI

QQC.Popup {
    id: popup;
    width: parent.width;
    implicitHeight: contentItem.implicitHeight + 4 * dpiScale;
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
        implicitHeight: contentHeight;
        QQC.ScrollIndicator.vertical: QQC.ScrollIndicator { }
        delegate: QQC.ItemDelegate {
            id: dlg;
            width: parent.width;
            implicitHeight: popup.itemHeight;

            contentItem: QQCI.IconLabel {
                anchors.fill: parent;
                text: modelData;
                icon.name: popup.icons[index] || "";
                icon.color: c;
                icon.height: popup.itemHeight / 2 + 5 * dpiScale;
                icon.width: popup.itemHeight / 2 + 5 * dpiScale;
                alignment: Qt.AlignLeft;
                leftPadding: 12 * dpiScale;
                rightPadding: 12 * dpiScale;
                color: c;
                property color c: popup.colors[index] || styleTextColor;
                topPadding: popup.itemHeight / 3.5;
                bottomPadding: popup.itemHeight / 3.5;

                font: popup.font;
                Component.onCompleted: { if (implicitWidth > popup.maxItemWidth) popup.maxItemWidth = implicitWidth; }
            }
            
            scale: dlg.down? 0.970 : 1.0;
            Ease on scale { }

            MouseArea { anchors.fill: parent; acceptedButtons: Qt.NoButton; cursorShape: Qt.PointingHandCursor; }
            onClicked: popup.clicked(index);

            background: Rectangle {
                color: dlg.hovered || dlg.highlighted? styleHighlightColor : "transparent";
                anchors.fill: parent;
                anchors.margins: 2 * dpiScale;
                radius: 4 * dpiScale;

                Rectangle {
                    x: 1 * dpiScale;
                    color: styleAccentColor;
                    height: parent.height * 0.45;
                    width: 3 * dpiScale;
                    radius: width;
                    y: (parent.height - height) / 2;
                    visible: lv.currentIndex === index;
                }
            }
            highlighted: popup.highlightedIndex === index;
        }
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
