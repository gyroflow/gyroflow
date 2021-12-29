import QtQuick 2.15

Column {
    spacing: parent.spacing;
    width: parent.width;

    default property alias data: advanced.data;

    LinkButton {
        text: qsTr("Advanced");
        anchors.horizontalCenter: parent.horizontalCenter;
        onClicked: advanced.opened = !advanced.opened;
    }
    Column {
        spacing: parent.spacing;
        id: advanced;
        property bool opened: false;
        width: parent.width;
        visible: opacity > 0;
        opacity: opened? 1 : 0;
        height: opened? implicitHeight : -10 * dpiScale;
        Ease on opacity { }
        Ease on height { id: anim; }
        onOpenedChanged: {
            anim.enabled = true;
            timer.start();
        }
        Timer {
            id: timer;
            interval: 700;
            onTriggered: anim.enabled = false;
        }
    }
}
