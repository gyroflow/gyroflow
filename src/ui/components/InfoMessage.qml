import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC

Rectangle {
    id: root;

    enum MsgType { Info, Warning, Error }
    property int type: InfoMessage.Warning;

    width: parent.width;
    height: t.height + 20 * dpiScale;
    color: type == InfoMessage.Warning? "#f6a10c" : 
           type == InfoMessage.Error?   "#f41717" : 
           type == InfoMessage.Info?    "#17b6f4" : 
           "transparent";
    radius: 5 * dpiScale;
    property alias text: t.text;
    property alias t: t;
    
    Text {
        id: t;
        font.pixelSize: 13 * dpiScale;
        font.family: styleFont;
        color: type == InfoMessage.Error? "#fff" : "#000";
        x: 15 * dpiScale;
        width: parent.width - 2*x;
        horizontalAlignment: Text.AlignHCenter;
        anchors.verticalCenter: parent.verticalCenter;
        wrapMode: Text.WordWrap;
    }
}
