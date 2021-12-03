import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC

import "../components/"

MenuItem {
    id: root;
    text: qsTr("Export settings");
    icon: "save";
    enabled: window.videoArea.vid.loaded;

    property int orgWidth: 0;
    property int orgHeight: 0;

    property int ratioWidth: orgWidth;
    property int ratioHeight: orgHeight;

    onOrgWidthChanged: { outputWidth.value = orgWidth; ratioWidth = orgWidth; }
    onOrgHeightChanged: { outputHeight.value = orgHeight; ratioHeight = orgHeight; }

    property alias outWidth: outputWidth.value;
    property alias outHeight: outputHeight.value;
    property alias codec: codec.currentText;
    property alias bitrate: bitrate.value;
    property alias gpu: gpu.checked;
    property alias audio: audio.checked;

    function updateOutputSize(isWidth) {
        if (lockAspectRatio.checked && ratioHeight > 0) {
            const ratio = ratioWidth / ratioHeight;
            if (isWidth) {
                outputHeight.preventChange2 = true;
                outputHeight.value = outputWidth.value / ratio;
                outputHeight.preventChange2 = false;
            } else {
                outputWidth.preventChange2 = true;
                outputWidth.value = outputHeight.value * ratio;
                outputWidth.preventChange2 = false;
            }
        }
        controller.set_output_size(outWidth, outHeight);
    }

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
            NumberField {
                property bool preventChange2: false;
                id: outputWidth;
                tooltip: qsTr("Width");
                width: 60 * dpiScale;
                onValueChanged: if (!preventChange2) root.updateOutputSize(true);
                live: false;
            }
            BasicText { leftPadding: 0; text: "x"; anchors.verticalCenter: parent.verticalCenter; }
            NumberField {
                property bool preventChange2: false;
                id: outputHeight;
                tooltip: qsTr("Height");
                width: 60 * dpiScale;
                onValueChanged: if (!preventChange2) root.updateOutputSize(false);
                live: false;
            }
            LinkButton {
                id: lockAspectRatio;
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
                display: QQC.Button.IconOnly;
                tooltip: qsTr("Lock aspect ratio");
                onCheckedChanged: if (checked) { ratioWidth = outWidth; ratioHeight = outHeight; }
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
