import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC

Button {
    id: root;
    // TODO popup direction
    property alias model: popup.model;

    rightPadding: 47 * dpiScale;

    Button {
        id: splitbtn;
        anchors.right: parent.right;
        width: 35 * dpiScale;
        height: parent.height;
        contentItem: Item { }
        accent: parent.accent;

        DropdownChevron { opened: popup.visible; color: root.textColor; anchors.centerIn: parent; }
        onClicked: popup.open();
    }
    Rectangle {
        anchors.left: splitbtn.left;
        width: 1 * dpiScale;
        height: parent.height;
        color: Qt.darker(styleAccentColor, 1.5);
    }
    Popup {
        id: popup;
        x: -width + parent.width;
        width: parent.width * 1.5;
        y: -height - 5 * dpiScale;
        currentIndex: -1;
    }

    Rectangle {
        anchors.fill: parent;
        border.width: 1 * dpiScale;
        border.color: Qt.darker(styleAccentColor, 1.5);
        radius: 6 * dpiScale;
        color: "transparent";
    }
}
