import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC

import "../components/"

MenuItem {
    text: qsTr("Export settings");
    icon: "save";

    property alias codec: codec.currentText;
    property alias outWidth: outputWidth.value;
    property alias outHeight: outputHeight.value;
    property alias bitrate: bitrate.value;
    property alias gpu: gpu.checked;
    property alias audio: audio.checked;

    ComboBox {
        id: codec;
        model: ["x264", "x265", "ProRes", "PNG sequence"];
        width: parent.width;
        currentIndex: 1;
    }
    Label {
        position: Label.Left;
        text: qsTr("Output size");
        Row {
            spacing: 5 * dpiScale;
            NumberField { id: outputWidth; tooltip: qsTr("Width"); width: 60 * dpiScale; }
            BasicText { leftPadding: 0; text: "x"; anchors.verticalCenter: parent.verticalCenter; }
            NumberField { id: outputHeight; tooltip: qsTr("Height"); width: 60 * dpiScale; }
            LinkButton {
                checked: true;
                height: parent.height * 0.75;
                icon.name: checked? "lock" : "unlocked";
                topPadding: 4 * dpiScale;
                bottomPadding: 4 * dpiScale;
                leftPadding: 3 * dpiScale;
                rightPadding: -3 * dpiScale;
                anchors.verticalCenter: parent.verticalCenter;
                onClicked: checked = !checked;
                textColor: checked? styleAccentColor : styleTextColor;
                display: QQC.Button.IconOnly
            }
        }
    }
    Label {
        position: Label.Left;
        text: qsTr("Bitrate");

        NumberField {
            id: bitrate;
            value: 0;
            unit: "Mbps";
            width: parent.width;
        }
    }

    CheckBox {
        id: gpu;
        text: qsTr("Use GPU encoding");
        checked: true;
    }
    CheckBox {
        id: audio;
        text: qsTr("Export audio");
        checked: true;
    }
}
