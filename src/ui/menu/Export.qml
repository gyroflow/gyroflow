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

    property real aspectRatio: 1.0;
    property alias outWidth: outputWidth.value;
    property alias outHeight: outputHeight.value;
    property alias defaultWidth: outputWidth.defaultValue;
    property alias defaultHeight: outputHeight.defaultValue;

    property alias outCodec: codec.currentText;
    property alias outBitrate: bitrate.value;
    property alias defaultBitrate: bitrate.defaultValue;
    property alias outGpu: gpu.checked;
    property alias outAudio: audio.checked;
    property string overridePixelFormat: "";
    property string outCodecOptions: "";

    property bool canExport: !resolutionWarning.visible && !resolutionWarning2.visible;

    Connections {
        target: controller;
        function onConvert_format(format, supported) {
            supported = supported.split(",").filter(v => !["CUDA", "D3D11", "BGRZ", "RGBZ", "BGRA", "UYVY422", "VIDEOTOOLBOX", "DXVA2", "MEDIACODEC", "VULKAN", "OPENCL", "QSV"].includes(v));
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

    property bool disableUpdate: false;
    function notifySizeChanged() {
        controller.set_output_size(outWidth, outHeight);
    }
    function ensureAspectRatio(byWidth) {
        if (lockAspectRatio.checked && aspectRatio > 0) {
            if (byWidth) {
                outHeight = Math.round(outWidth / aspectRatio);
            } else {
                outWidth = Math.round(outHeight * aspectRatio);
            }
        }
    }
    function setDefaultSize(w, h) {
        aspectRatio   = w / h;
        defaultWidth  = w;
        defaultHeight = h;

        disableUpdate = true;
        outWidth      = w;
        outHeight     = h;
        disableUpdate = false;
    }
    function videoInfoLoaded(w, h, br) {
        setDefaultSize(w, h);
        Qt.callLater(notifySizeChanged);

        outBitrate     = br;
        defaultBitrate = br;

        codec.updateGpuStatus();
    }
    function lensProfileLoaded(w, h) {
        setDefaultSize(w, h);
        Qt.callLater(notifySizeChanged);
    }

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
        Item {
            width: parent.width;
            height: outputWidth.height;
            NumberField {
                id: outputWidth;
                tooltip: qsTr("Width");
                anchors.verticalCenter: parent.verticalCenter;               
                anchors.left: parent.left;
                width: (sizeMenuBtn.x - outputHeight.anchors.rightMargin - x - lockAspectRatio.width) / 2 - lockAspectRatio.anchors.leftMargin;
               // width: 60 * dpiScale;
                intNoThousandSep: true;
                reset: () => { aspectRatio = defaultValue / Math.max(1,outHeight); value = defaultValue; };
                onValueChanged: {
                    if (!disableUpdate) {
                        disableUpdate = true;
                        ensureAspectRatio(true);
                        Qt.callLater(notifySizeChanged);
                        disableUpdate = false;
                    }
                }
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
                anchors.left: outputWidth.right;
                anchors.leftMargin: 5 * dpiScale;
                onClicked: checked = !checked;
                textColor: checked? styleAccentColor : styleTextColor;
                display: QQC.Button.IconOnly;
                tooltip: qsTr("Lock aspect ratio");
                onCheckedChanged: if (checked) { aspectRatio = outWidth / Math.max(1,outHeight); }
            }
            NumberField {
                id: outputHeight;
                tooltip: qsTr("Height");
                intNoThousandSep: true;
                anchors.verticalCenter: parent.verticalCenter;
                anchors.right: sizeMenuBtn.left;
                anchors.rightMargin: 5 * dpiScale;
                width: outputWidth.width;
                onValueChanged: {
                    if (!disableUpdate) {
                        disableUpdate = true;
                        ensureAspectRatio(false);
                        Qt.callLater(notifySizeChanged);
                        disableUpdate = false;
                    }
                }
                live: false;
                reset: () => { aspectRatio = outWidth / Math.max(1,defaultValue); value = defaultValue; };
            }
            LinkButton {
                id: sizeMenuBtn;
                height: parent.height;
                icon.name: "settings";
                leftPadding: 3 * dpiScale;
                rightPadding: 3 * dpiScale;
                anchors.verticalCenter: parent.verticalCenter;
                anchors.right: parent.right;
                display: QQC.Button.IconOnly;
                tooltip: qsTr("Output size preset");
                onClicked: sizeMenu.popup(x, y + height);
            }
            Menu {
                id: sizeMenu;
                font.pixelSize: 11.5 * dpiScale;

                function setSize(w, h) {
                    disableUpdate = true;
                    aspectRatio = w / h;
                    outWidth = w;
                    outHeight = h;
                    Qt.callLater(notifySizeChanged);
                    disableUpdate = false;
                }

                Action { text: "8k (7680 x 4320)";     onTriggered: sizeMenu.setSize(7680, 4320) }
                Action { text: "6k (6016 × 3384)";     onTriggered: sizeMenu.setSize(6016, 3384) }
                Action { text: "4k (3840 x 2160)";     onTriggered: sizeMenu.setSize(3840, 2160) }
                Action { text: "2k (2048 x 1080)";     onTriggered: sizeMenu.setSize(2048, 1080) }
                QQC.MenuSeparator { verticalPadding: 5 * dpiScale; }
                Action { text: "1440p (2560 x 1440)";  onTriggered: sizeMenu.setSize(2560, 1440) }
                Action { text: "1080p (1920 x 1080)";  onTriggered: sizeMenu.setSize(1920, 1080) }
                Action { text: "720p (1280 x 720)";    onTriggered: sizeMenu.setSize(1280, 720) }
                Action { text: "480p (640 x 480)";     onTriggered: sizeMenu.setSize( 640, 480) }
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
