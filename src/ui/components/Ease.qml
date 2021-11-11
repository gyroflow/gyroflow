import QtQuick 2.15;

Behavior {
    id: ee;
    property int duration: 700;
    NumberAnimation { 
        duration: ee.duration;
        easing.type: Easing.OutExpo;
    }
}