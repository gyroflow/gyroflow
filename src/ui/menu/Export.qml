// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC
import Qt.labs.settings

import "../components/"

MenuItem {
    id: root;
    text: qsTr("Export settings");
    icon: "save";
    innerItem.enabled: window.videoArea.vid.loaded;

    function updateCodecParams() {
        codec.currentIndexChanged();
    }

    property bool isOsx: Qt.platform.os == "osx";

    property var exportFormats: [
        { "name": "x264",          "max_size": [4096, 2160], "extension": ".mp4",      "gpu": true,  "audio": true,  "variants": [ ] },
        { "name": "x265",          "max_size": [8192, 4320], "extension": ".mp4",      "gpu": true,  "audio": true,  "variants": [ ] },
        { "name": "ProRes",        "max_size": [8192, 4320], "extension": ".mov",      "gpu": isOsx, "audio": true,  "variants": ["Proxy", "LT", "Standard", "HQ", "4444", "4444XQ"] },
        { "name": "EXR Sequence",  "max_size": false,        "extension": "_%05d.exr", "gpu": false, "audio": false, "variants": [] },
        { "name": "PNG Sequence",  "max_size": false,        "extension": "_%05d.png", "gpu": false, "audio": false, "variants": ["8-bit", "16-bit"] },
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
    property alias outCodec: codec.currentText;
    property alias outBitrate: bitrate.value;
    property alias outGpu: gpu.checked;
    property alias outAudio: audio.checked;
    property string overridePixelFormat: "";
    property string outCodecOptions: "";

    onOutWidthChanged: Qt.callLater(applyOutputSize);
    onOutHeightChanged: Qt.callLater(applyOutputSize);

    Connections {
        target: controller;
        function onConvert_format(format, supported) {
            supported = supported.split(",").filter(v => !["CUDA", "D3D11", "BGRZ", "RGBZ", "VIDEOTOOLBOX", "DXVA2", "MEDIACODEC", "VULKAN", "OPENCL", "QSV"].includes(v));
            let buttons = supported.map(f => ({
                text: f,
                clicked: () => {
                    overridePixelFormat = f;
                    window.renderBtn.render();
                }
            }));
            buttons.push({
                text: qsTr("Render using CPU"),
                accent: true,
                clicked: () => {
                    gpu.checked = false;
                    window.renderBtn.render();
                }
            });
            buttons.push({ text: qsTr("Cancel") });

            messageBox(Modal.Question, qsTr("GPU accelerated encoder doesn't support this pixel format (%1).\nDo you want to convert to a different supported pixel format or keep the original one and render on the CPU?").arg(format), buttons);
        }
    }

    function applyOutputSize() {
        controller.set_output_size(outWidth, outHeight);
    }
    function setOutputSize(w, h) {
        orgWidth = w;
        orgHeight = h;
        outputHeight.preventChange2 = true;
        outputWidth.preventChange2 = true;
        outputWidth.value = w;
        outputHeight.value = h;
        outputWidth.preventChange2 = false;
        outputHeight.preventChange2 = false;
    }
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
        Qt.callLater(applyOutputSize);
    }
    function vidInfoLoaded() { codec.updateGpuStatus(); }

    ComboBox {
        id: codec;
        model: exportFormats.map(x => x.name);
        width: parent.width;
        currentIndex: 1;
        function updateExtension(ext) {
            window.outputFile = window.outputFile.replace(/(_%[0-9d]+)?\.[a-z0-9]+$/i, ext);
        }
        function updateGpuStatus() {
            const format = exportFormats[currentIndex];
            gpu.enabled2 = format.gpu;
            if ((format.name == "x264" && window.vidInfo.pixelFormat.includes("10 bit"))) {
                gpu.enabled2 = false;
            }
            gpu.checked = gpu.enabled2;
        }
        onCurrentIndexChanged: {
            const format = exportFormats[currentIndex];
            audio.enabled2 = format.audio;
            if (!audio.enabled2) audio.checked = false;

            updateGpuStatus();
            updateExtension(format.extension);
        }
    }
    ComboBox {
        model: exportFormats[codec.currentIndex].variants;
        width: parent.width;
        visible: model.length > 0;
        onVisibleChanged: if (!visible) { root.outCodecOptions = ""; } else { root.outCodecOptions = currentText; }
        onCurrentTextChanged: root.outCodecOptions = currentText;
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
                intNoThousandSep: true;
                onValueChanged: if (!preventChange2) root.updateOutputSize(true);
                live: false;
            }
            BasicText { leftPadding: 0; text: "x"; anchors.verticalCenter: parent.verticalCenter; }
            NumberField {
                property bool preventChange2: false;
                id: outputHeight;
                tooltip: qsTr("Height");
                width: 60 * dpiScale;
                intNoThousandSep: true;
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
            defaultValue: 20;
            unit: qsTr("Mbps");
            width: parent.width;
        }
    }

    CheckBox {
        id: gpu;
        text: qsTr("Use GPU encoding");
        checked: true;
        property bool enabled2: true;
        enabled: enabled2;
        tooltip: enabled2? qsTr("GPU encoders typically generate output of lower quality than software encoders, but are significantly faster.") + "\n" + 
                           qsTr("They require a higher bitrate to make output with the same perceptual quality, or they make output with a lower perceptual quality at the same bitrate.") + "\n" + 
                           qsTr("Uncheck this option for maximum possible quality.")
                         :
                           qsTr("GPU acceleration is not available for the pixel format of this video.");
    }
    CheckBox {
        id: audio;
        text: qsTr("Export audio");
        checked: true;
        property bool enabled2: true;
        enabled: enabled2;
    }
}
