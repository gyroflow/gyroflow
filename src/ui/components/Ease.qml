import QtQuick 2.15;

Behavior {
    id: ee;
    property int duration: 700;
    property alias type: anim.easing.type;
    NumberAnimation {
        id: anim;
        duration: ee.duration;
        easing.type: Easing.OutExpo;
    }
}