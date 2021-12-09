import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC
import Qt.labs.settings 1.0

import "../components/"

MenuItem {
    id: root;
    text: qsTr("Export settings");
    icon: "save";
    innerItem.enabled: window.videoArea.vid.loaded;

    function updateCodecParams() {
        codec.currentIndexChanged();
    }

    property var exportFormats: [
        { "name": "x264",          "max_size": [4096, 2160], "variants": [ ] },
        { "name": "x265",          "max_size": [8192, 4320], "variants": [ ] },
        { "name": "ProRes",        "max_size": [8192, 4320], "variants": ["Proxy", "LT", "Standard", "HQ", "4444", "4444XQ"] },
        { "name": "PNG Sequence",  "max_size": false,        "variants": ["8-bit", "16-bit"] },
    ];

    Settings {
        property alias defaultCodec: codec.currentIndex;
        property alias exportGpu: gpu.checked;
        property alias exportAudio: audio.checked;
    }

    property int orgWidth: 0;
    property int orgHeight: 0;

    property int ratioWidth: orgWidth;
    property int ratioHeight: orgHeight;

    onOrgWidthChanged: {
        outputWidth.preventChange2 = true;
        outputWidth.value = orgWidth;
        ratioWidth = orgWidth;
        outputWidth.preventChange2 = false;
    }
    onOrgHeightChanged: {
        outputHeight.preventChange2 = true;
        outputHeight.value = orgHeight;
        ratioHeight = orgHeight;
        outputHeight.preventChange2 = false;
    }

    property bool canExport: !resolutionWarning.visible && !resolutionWarning2.visible;

    property int outWidth: outputWidth.value;
    property int outHeight: outputHeight.value;
    property alias codec: codec.currentText;
    property alias bitrate: bitrate.value;
    property alias gpu: gpu.checked;
    property alias audio: audio.checked;
    property string codecOptions: "";

    function updateOutputSize(isWidth) {
        if (lockAspectRatio.checked && ratioHeight > 0) {
            const ratio = ratioWidth / ratioHeight;
            if (isWidth) {
                outputHeight.preventChange2 = true;
                outputHeight.value = Math.round(outputWidth.value / ratio);
                outputHeight.preventChange2 = false;
            } else {
                outputWidth.preventChange2 = true;
                outputWidth.value = Math.round(outputHeight.value * ratio);
                outputWidth.preventChange2 = false;
            }
        }
        controller.set_output_size(outWidth, outHeight);
    }

    ComboBox {
        id: codec;
        model: exportFormats.map(x => x.name);
        width: parent.width;
        currentIndex: 1;
        function updateExtension(ext) {
            window.outputFile = window.outputFile.replace(/(_%.+)?\.[a-z0-9]+$/i, ext);
        }
        onCurrentIndexChanged: {
            gpu.enabled = currentIndex == 0 || currentIndex == 1;
            audio.enabled = currentIndex != 3;
            if (!gpu.enabled) gpu.checked = false;
            if (!audio.enabled) audio.checked = false;

            if (currentIndex == 2) updateExtension(".mov");
            else if (currentIndex == 3) updateExtension("_%05d.png");
            else updateExtension(".mp4");
        }
    }
    ComboBox {
        model: exportFormats[codec.currentIndex].variants;
        width: parent.width;
        visible: model.length > 0;
        onVisibleChanged: if (!visible) { root.codecOptions = ""; } else { root.codecOptions = currentText; }
        onCurrentTextChanged: root.codecOptions = currentText;
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

    InfoMessageSmall {
        id: resolutionWarning;
        type: InfoMessage.Error;
        property var maxSize: exportFormats[codec.currentIndex].max_size;
        show: maxSize && (outWidth > maxSize[0] || outHeight > maxSize[1]);
        text: qsTr("This resolution is not supported by the selected codec.") + "\n" + 
              qsTr("Maximum supported resolution is %1.").arg(maxSize? maxSize.join("x") : ""); 
    }
    InfoMessageSmall {
        id: resolutionWarning2;
        type: InfoMessage.Error;
        show: (outWidth % 2) != 0 || (outHeight % 2) != 0;
        text: qsTr("Resolution must be divisible by 2."); 
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
        tooltip: qsTr("GPU encoders typically generate output of lower quality than software encoders, but are significantly faster.") + "\n" + 
                 qsTr("They require a higher bitrate to make output with the same perceptual quality, or they make output with a lower perceptual quality at the same bitrate.") + "\n" + 
                 qsTr("Uncheck this option for maximum possible quality.");
    }
    CheckBox {
        id: audio;
        text: qsTr("Export audio");
        checked: true;
    }
}
