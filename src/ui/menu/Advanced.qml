// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import Qt.labs.settings

import "../components/"

MenuItem {
    text: qsTr("Advanced");
    iconName: "settings";
    opened: false;
    objectName: "advanced";

    Settings {
        id: settings;
        property alias previewPipeline: previewPipeline.currentIndex;
        property alias renderBackground: renderBackground.text;
        property alias uiScaling: uiScaling.currentIndex;
        property alias safeAreaGuide: safeAreaGuide.checked;
        property alias gpudecode: gpudecode.checked;
        property alias backgroundMode: backgroundMode.currentIndex;
        property alias marginPixels: marginPixels.value;
        property alias featherPixels: featherPixels.value;
        property alias defaultSuffix: defaultSuffix.text;
        property alias playSounds: playSounds.checked;
        property alias r3dConvertFormat: r3dConvertFormat.currentIndex;
        property alias r3dColorMode: r3dColorMode.currentIndex;
        property alias r3dGammaCurve: r3dGammaCurve.currentIndex;
        property alias r3dColorSpace: r3dColorSpace.currentIndex;
        property alias r3dRedlineParams: r3dRedlineParams.text;
        property string lang: ui_tools.get_default_language();
    }
    property alias defaultSuffix: defaultSuffix;
    property alias previewResolution: previewResolution.currentIndex;
    property alias r3dConvertFormat: r3dConvertFormat;
    property alias gpudecode: gpudecode;

    function loadGyroflow(obj) {
        if (obj.hasOwnProperty("background_mode")) backgroundMode.currentIndex = +obj.background_mode;
        if (obj.hasOwnProperty("background_margin")) marginPixels.value = +obj.background_margin;
        if (obj.hasOwnProperty("background_margin_feather")) featherPixels.value = +obj.background_margin_feather;
        if (obj.hasOwnProperty("background_color")) renderBackground.text = Qt.rgba(obj.background_color[0], obj.background_color[1], obj.background_color[2], obj.background_color[3]).toString();
    }
    Label {
        position: Label.LeftPosition;
        text: qsTr("Preview resolution");

        ComboBox {
            id: previewResolution;
            model: [QT_TRANSLATE_NOOP("Popup", "Full"), "4k", "1080p", "720p", "480p"];
            font.pixelSize: 12 * dpiScale;
            width: parent.width;
            currentIndex: 0;
            Component.onCompleted: {
                if (settings.value("previewResolution", -1) != -1)
                    currentIndex = +settings.value("previewResolution", -1);
            }
            onCurrentIndexChanged: {
                let target_height = -1; // Full
                switch (currentIndex) {
                    case 0: window.videoArea.vid.setProperty("scale", ""); break;
                    case 1: target_height = 2160; window.videoArea.vid.setProperty("scale", "3840x2160"); break;
                    case 2: target_height = 1080; window.videoArea.vid.setProperty("scale", "1920x1080"); break;
                    case 3: target_height = 720;  window.videoArea.vid.setProperty("scale", "1280x720");  break;
                    case 4: target_height = 480;  window.videoArea.vid.setProperty("scale", "640x480");   break;
                }

                controller.set_preview_resolution(target_height, window.videoArea.vid);
                settings.setValue("previewResolution", currentIndex);
            }
        }
    }

    Label {
        position: Label.LeftPosition;
        text: qsTr("Background mode");
        ComboBox {
            id: backgroundMode;
            model: [QT_TRANSLATE_NOOP("Popup", "Solid color"), QT_TRANSLATE_NOOP("Popup", "Repeat edge pixels"), QT_TRANSLATE_NOOP("Popup", "Mirror edge pixels"), QT_TRANSLATE_NOOP("Popup", "Margin with feather")];
            font.pixelSize: 12 * dpiScale;
            width: parent.width;
            currentIndex: 0;
            onCurrentIndexChanged: controller.background_mode = currentIndex;
        }
    }
    Column {
        width: parent.width;
        visible: backgroundMode.currentIndex == 3;
        Label {
            text: qsTr("Margin");
            SliderWithField {
                id: marginPixels;
                value: 0.20;
                defaultValue: 20;
                from: 0;
                to: 50;
                unit: "%";
                precision: 0;
                width: parent.width;
                keyframe: "BackgroundMargin";
                scaler: 100.0;
                onValueChanged: controller.background_margin = value;
            }
        }
        Label {
            text: qsTr("Feather");
            SliderWithField {
                id: featherPixels;
                value: 0.05;
                defaultValue: 5;
                from: 0;
                to: 50;
                unit: "%";
                precision: 0;
                width: parent.width;
                keyframe: "BackgroundFeather";
                scaler: 100.0;
                onValueChanged: controller.background_margin_feather = value;
            }
        }
    }
    Label {
        position: Label.LeftPosition;
        visible: backgroundMode.currentIndex == 0;
        text: qsTr("Render background");

        TextField {
            id: renderBackground;
            text: "#111111";
            width: parent.width;
            onTextChanged: controller.set_background_color(text, window.videoArea.vid);
        }
    }
    Label {
        position: Label.LeftPosition;
        text: qsTr("Theme");

        ComboBox {
            id: themeList;
            model: [];
            font.pixelSize: 12 * dpiScale;
            width: parent.width;
            Component.onCompleted: {
                const savedTheme = +settings.value("theme", 1);
                let m = [QT_TRANSLATE_NOOP("Popup", "Light"), QT_TRANSLATE_NOOP("Popup", "Dark")];
                if (!(isMobile && screenSize < 7.0)) {
                    m.push(QT_TRANSLATE_NOOP("Popup", "Mobile Light"));
                    m.push(QT_TRANSLATE_NOOP("Popup", "Mobile Dark"));
                }
                model = m;
                currentIndex = savedTheme;
            }
            onCurrentIndexChanged: {
                const themes = ["light", "dark", "mobile_light", "mobile_dark"];
                let theme = themes[currentIndex];
                ui_tools.set_theme(theme);
                settings.setValue("theme", currentIndex);
            }
        }
    }
    Label {
        position: Label.LeftPosition;
        text: qsTr("UI scaling");
        ComboBox {
            id: uiScaling;
            model: ["50%", "75%", "100%", "125%", "150%", "175%", "200%"];
            font.pixelSize: 12 * dpiScale;
            width: parent.width;
            currentIndex: 2;
            onCurrentIndexChanged: {
                ui_tools.set_scaling([0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0][currentIndex]);
            }
        }
    }
    Label {
        position: Label.LeftPosition;
        text: qsTr("Language");

        ComboBox {
            id: langList;
            property var langs: [
                ["English",                      "en"],
                ["Chinese - Simplified (简体中文)",  "zh_CN"],
                ["Chinese - Traditional (繁體中文)", "zh_TW"],
                ["Czech (Čeština)",              "cs"],
                ["Danish (dansk)",               "da"],
                ["Finnish (suomi)",              "fi"],
                ["French (français)",            "fr"],
                ["Galician (Galego)",            "gl"],
                ["German (Deutsch)",             "de"],
                ["Greek (Ελληνικά)",             "el"],
                ["Indonesian (Bahasa Indonesia)","id"],
                ["Italian (italiano)",           "it"],
                ["Japanese (日本語)",             "ja"],
                ["Korean (한국어)",              "ko"],
                ["Norwegian (norsk)",            "no"],
                ["Polish (polski)",              "pl"],
                ["Portuguese - Brazilian (português brasileiro)", "pt_BR"],
                ["Portuguese (português)",       "pt"],
                ["Russian (русский)",            "ru"],
                ["Slovak (slovenský)",           "sk"],
                ["Spanish (español)",            "es"],
                ["Turkish (Türkçe)",             "tr"],
                ["Ukrainian (Українська мова)",  "uk"]
            ];
            Component.onCompleted: {
                let selectedIndex = 0;
                let i = 0;
                model = langs.map((x) => { if (x[1] == settings.lang) { selectedIndex = i; } i++; return x[0]; });
                currentIndex = selectedIndex;
            }
            font.pixelSize: 12 * dpiScale;
            width: parent.width;
            function setLang() {
                settings.lang = langs[currentIndex][1];

                window.LayoutMirroring.enabled = settings.lang == "ar" || settings.lang == "fa" || settings.lang == "he";
                window.LayoutMirroring.childrenInherit = true;
                ui_tools.set_language(settings.lang);
            }
            onCurrentIndexChanged: Qt.callLater(setLang);
        }
    }
    CheckBox {
        id: safeAreaGuide;
        text: qsTr("Safe area guide");
        tooltip: qsTr("When FOV > 1, show an rectangle simulating FOV = 1 over the preview video.\nNote that this is only a visual indicator, it doesn't affect rendering.");
        checked: false;
        onCheckedChanged: controller.show_safe_area = checked;
        Component.onCompleted: Qt.callLater(checkedChanged);
    }
    CheckBox {
        id: gpudecode;
        text: qsTr("Use GPU decoding");
        checked: true;
        onCheckedChanged: controller.set_gpu_decoding(checked);
    }
    Label {
        id: r3dConvertFormatLabel;
        position: Label.LeftPosition;
        text: qsTr("Format for R3D conversion");
        visible: !!controller.find_redline();
        ComboBox {
            id: r3dConvertFormat;
            model: [
                "ProRes 422 HQ",
                "ProRes 422",
                "ProRes 422 LT",
                "ProRes 422 Proxy",
                "ProRes 4444",
                "ProRes 4444 XQ",
            ];
            font.pixelSize: 12 * dpiScale;
            width: parent.width;
        }
    }
    Label {
        position: Label.LeftPosition;
        text: "Colors for R3D conversion";
        visible: r3dConvertFormatLabel.visible;
        ComboBox {
            id: r3dColorMode;
            model: ["Fully graded in REDCINE-X", "Primary development only"];
            font.pixelSize: 12 * dpiScale;
            width: parent.width;
        }
    }
    Label {
        position: Label.LeftPosition;
        text: "Gamma curve for R3D conversion";
        visible: r3dConvertFormatLabel.visible;
        ComboBox {
            id: r3dGammaCurve;
            model: ["Linear", "BT.709", "sRGB", "REDlog", "PDLog985", "PDLog685", "PDLogCustom", "REDspace", "REDgamma", "REDLogFilm", "REDgamma2", "REDgamma3", "REDgamma4", "ST 2084", "BT.1886", "Log3G12", "Log3G10", "Hybrid Log-Gamma", "Gamma 2.2", "Gamma 2.6"];
            currentIndex: 7;
            font.pixelSize: 12 * dpiScale;
            width: parent.width;
        }
    }
    Label {
        position: Label.LeftPosition;
        text: "Color space for R3D conversion";
        visible: r3dConvertFormatLabel.visible;
        ComboBox {
            id: r3dColorSpace;
            model: [ "REDspace", "CameraRGB", "BT.709", "REDcolor", "sRGB", "Adobe1998", "REDcolor2", "REDcolor3", "DRAGONcolor", "XYZ", "REDcolor4", "DRAGONcolor2", "BT.2020", "REDWideGamutRGB", "DCI-P3", "DCI-P3 D65"];
            currentIndex: 0;
            font.pixelSize: 12 * dpiScale;
            width: parent.width;
        }
    }
    Label {
        position: Label.LeftPosition;
        text: "Additional REDline params";
        visible: r3dConvertFormatLabel.visible;
        TextField {
            id: r3dRedlineParams;
            width: parent.width;
        }
    }
    Label {
        position: Label.LeftPosition;
        text: qsTr("Preview pipeline");

        ComboBox {
            id: previewPipeline;
            model: ["Zero-copy Qt RHI", "Zero-copy OpenCL", "OpenCL/wgpu/CPU"];
            font.pixelSize: 12 * dpiScale;
            width: parent.width;
            currentIndex: 0;
            onCurrentIndexChanged: {
                if (currentIndex != 2) {
                    if (previewResolution.currentIndex == 3) {
                        previewResolution.currentIndex = 0;
                        Qt.callLater(window.exportSettings.notifySizeChanged);
                    }
                }
                controller.set_preview_pipeline(currentIndex);
                Qt.callLater(processingDevice.updateController);
                Qt.callLater(window.videoArea.vid.forceRedraw);
            }
            Component.onCompleted: Qt.callLater(currentIndexChanged);
        }
    }
    Label {
        position: Label.TopPosition;
        text: qsTr("Device for video processing");
        visible: processingDevice.model.length > 0;
        ComboBox {
            id: processingDevice;
            model: [];
            font.pixelSize: 12 * dpiScale;
            width: parent.width;
            currentIndex: 0;
            property bool preventChange: true;
            Connections {
                target: controller;
                function onGpu_list_loaded(list) {
                    const saved = settings.value("processingDevice", defaultInitializedDevice);
                    processingDevice.preventChange = true;
                    processingDevice.model = [...list, qsTr("CPU only")];
                    for (let i = 0; i < list.length; ++i) {
                        if (list[i] == saved) {
                            processingDevice.currentIndex = i;
                            break;
                        }
                    }
                    if (saved != defaultInitializedDevice) {
                        Qt.callLater(processingDevice.updateController);
                    }
                    processingDevice.preventChange = false;
                    if (saved == "cpu") {
                        processingDevice.currentIndex = processingDevice.model.length - 1;
                    }
                }
            }
            Component.onCompleted: controller.list_gpu_devices();
            onCurrentIndexChanged: {
                if (preventChange) return;
                Qt.callLater(processingDevice.updateController);
            }
            function updateController() {
                if (model.length == 0) return;
                if (currentIndex == model.length - 1) {
                    controller.set_device(-1);
                } else {
                    controller.set_device(currentIndex);
                }
                const text = currentIndex == model.length - 1? "cpu" : currentText;
                settings.setValue("processingDevice", text);
                settings.setValue("processingDeviceIndex", processingDevice.currentIndex);
            }
        }
    }
    BasicText {
        visible: text.length > 0;
        text: controller.processing_info;
        width: parent.width;
        wrapMode: Text.WordWrap;
        font.pixelSize: 11 * dpiScale;
    }
    Label {
        position: Label.LeftPosition;
        text: qsTr("Default file suffix");

        TextField {
            id: defaultSuffix;
            text: "_stabilized";
            width: parent.width;
            onTextChanged: render_queue.default_suffix = text;
        }
    }
    CheckBox {
        id: playSounds;
        text: qsTr("Notification sounds");
        checked: true;
    }
    Item { width: 1; height: 10 * dpiScale; }
    LinkButton {
        text: qsTr("Reset all settings to default");
        textColor: "#f67575"
        anchors.horizontalCenter: parent.horizontalCenter;
        onClicked: {
            messageBox(Modal.Warning, qsTr("Are you sure you want to clear all settings and restore the defaults?"), [
                { text: qsTr("Yes"), clicked: () => {
                    // Preserve lens profile favorites
                    const lenses = window.settings.value("lensProfileFavorites");

                    controller.clear_settings();

                    if (lenses) window.settings.setValue("lensProfileFavorites", lenses);

                    messageBox(Modal.Info, qsTr("Settings cleared, please restart Gyroflow for the changes to take effect."), [
                        { text: qsTr("Exit"), accent: true, clicked: Qt.quit},
                        { text: qsTr("Ok") },
                    ]);
                }},
                { text: qsTr("No"), accent: true },
            ]);
        }
    }
}
