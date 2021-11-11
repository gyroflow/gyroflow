import QtQuick 2.15

Rectangle {
    property QtObject timeline: null;
    property int org_timestamp_us: 0;
    property real position: 0;
    property real offsetMs: 0;

    id: root;

    x: timeline.mapToVisibleArea(position) * (parent.width);
    y: 35 * dpiScale;
    radius: width;
    height: parent.height - 25 * dpiScale;
    width: 1 * dpiScale;
    color: "#dcae24";
    visible: x >= 0 && x <= parent.width;

    signal edit(real timestamp_us, real offset_ms);
    signal remove(real timestamp_us);

    Rectangle {
        height: 12 * dpiScale;
        width: 13 * dpiScale;
        color: "#dcae24";
        radius: 3 * dpiScale;
        //y: -5 * dpiScale;
        x: -width / 2;
        anchors.bottom: parent.bottom;
        opacity: ma.containsMouse? 0.8 : 1.0;

        Rectangle {
            height: 11 * dpiScale;
            width: 11 * dpiScale;
            color: parent.color;
            radius: 3 * dpiScale;
            anchors.centerIn: parent;

            anchors.verticalCenterOffset: -3 * dpiScale;
            rotation: 45;
        }
        MouseArea {
            id: ma;
            hoverEnabled: true;
            anchors.fill: parent;
            anchors.margins: -15 * dpiScale;
            cursorShape: Qt.PointingHandCursor;
            onClicked: (mouse) => {
                if (mouse.button === Qt.LeftButton) {
                    root.edit(root.org_timestamp_us, root.offsetMs);
                } else {
                    popup.open();
                }
            }
            acceptedButtons: Qt.LeftButton | Qt.RightButton
        }
        BasicText {
            leftPadding: 0;
            text: root.offsetMs.toFixed(2) + " ms";
            anchors.horizontalCenter: parent.horizontalCenter;
            y: 16 * dpiScale;
            font.pixelSize: 11 * dpiScale;
        }

        Popup {
            id: popup;
            width: maxItemWidth + 10 * dpiScale;
            y: -height - 5 * dpiScale;
            currentIndex: -1;
            model: [qsTr("Edit offset"), qsTr("Delete sync point")];
            icons: ["pencil", "bin"];
            colors: [styleTextColor, "#f67575"];
            itemHeight: 27 * dpiScale;
            font.pixelSize: 11.5 * dpiScale;
            onClicked: (index) => {
                popup.close();
                switch (index) {
                    case 0: root.edit(root.org_timestamp_us, root.offsetMs); break;
                    case 1: root.remove(root.org_timestamp_us); break;
                }
            }
        }
    }
}
