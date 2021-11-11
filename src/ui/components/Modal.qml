import QtQuick 2.15

Rectangle {
    id: root;
    property alias text: t.text;
    property alias buttons: btns.model;
    property bool opened: false;
    property int accentButton: -1;

    signal clicked(int index);

    anchors.fill: parent;
    color: "#80000000";
    opacity: pp.opacity;
    visible: opacity > 0;

    MouseArea { visible: root.opened; anchors.fill: parent; preventStealing: true; hoverEnabled: true; }
    Rectangle {
        id: pp;
        anchors.centerIn: parent;
        anchors.verticalCenterOffset: root.opened? 0 : -50 * dpiScale;
        Ease on anchors.verticalCenterOffset { }
        Ease on opacity { }
        opacity: root.opened? 1 : 0;
        width: 400;
        height: col.height + 30 * dpiScale;
        property real offs: 0;
        color: styleBackground2;
        radius: 7 * dpiScale;
        border.width: 1 * dpiScale;
        border.color: styleSliderHandle;


        Column {
            id: col;
            width: parent.width;
            anchors.centerIn: parent;
            Item { height: 10 * dpiScale; width: 1; }
            BasicText {
                id: t;
                width: parent.width;
                horizontalAlignment: Text.AlignHCenter;
                wrapMode: Text.WordWrap;
                font.pixelSize: 14 * dpiScale;
            }
            Item { height: 25 * dpiScale; width: 1; }
            Row {
                anchors.horizontalCenter: parent.horizontalCenter;
                spacing: 10 * dpiScale;
                Repeater {
                    id: btns;
                    Button {
                        text: modelData;
                        onClicked: root.clicked(index);
                        leftPadding: 20 * dpiScale;
                        rightPadding: 20 * dpiScale;
                        accent: root.accentButton === index;
                    }
                }
            }
        }
    }
}
