import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC

Rectangle {
    id: root;
    width: parent.width;
    height: t.height + 20 * dpiScale;
    color: "#f6a10c";
    radius: 5 * dpiScale;
    property alias text: t.text;
    property alias t: t;
    
    Text {
        id: t;
        font.pixelSize: 13 * dpiScale;
        font.family: styleFont;
        color: "#000";
        x: 15 * dpiScale;
        width: parent.width - 2*x;
        horizontalAlignment: Text.AlignHCenter;
        anchors.verticalCenter: parent.verticalCenter;
        wrapMode: Text.WordWrap;
    }
}
