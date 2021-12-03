import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC

QQC.Slider {
    id: slider;
    height: 20 * dpiScale;
    opacity: enabled? 1.0 : 0.5;
    property string unit: "";
    property int precision: 3;
    background: Rectangle {
        x: parent.leftPadding
        y: parent.topPadding + parent.availableHeight / 2 - height / 2
        width: parent.availableWidth
        height: 4 * dpiScale;
        radius: 4 * dpiScale;
        color: styleSliderBackground;

        Rectangle {
            width: parent.parent.visualPosition * parent.width
            height: parent.height
            color: styleAccentColor
            radius: parent.radius
        }
    }
    handle: Rectangle {
        x: parent.leftPadding + parent.visualPosition * (parent.availableWidth) - width/2
        y: parent.topPadding + parent.availableHeight / 2 - height / 2
        radius: width;
        height: parent.height * 0.9;
        width: height;
        anchors.verticalCenter: parent.verticalCenter;
        color: styleSliderHandle;
        Rectangle {
            radius: width;
            height: parent.height * 0.7;
            scale: (parent.parent.pressed? 1.1 : parent.parent.hovered? 0.9 : 1.0);
            Ease on scale { duration: 200; }
            width: height;
            anchors.centerIn: parent;
            color: styleAccentColor
        }
    }

    ToolTip {
        delay: 0;
        parent: handle;
        visible: slider.pressed;
        text: slider.valueAt(slider.position).toFixed(slider.precision) + (slider.unit? " " + slider.unit : "");
        bottomMargin: 5 * dpiScale;
    }
}
