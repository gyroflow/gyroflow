// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import Qt.labs.settings

import "../components/"

MenuItem {
    text: qsTr("Advanced");
    icon: "settings";
    opened: false;
    objectName: "advanced";

    Settings {
        id: settings;
        property alias previewResolution: previewResolution.currentIndex;
        property alias renderBackground: renderBackground.text;
        property alias theme: themeList.currentIndex;
        property alias uiScaling: uiScaling.currentIndex;
        property alias safeAreaGuide: safeAreaGuide.checked;
        property alias gpudecode: gpudecode.checked;
        property alias backgroundMode: backgroundMode.currentIndex;
        property alias marginPixels: marginPixels.value;
        property alias featherPixels: featherPixels.value;
        property string lang: ui_tools.get_default_language();
    }

    function loadGyroflow(obj) {
        if (obj.background_mode) backgroundMode.currentIndex = obj.background_mode;
        if (obj.background_margin) marginPixels.value = obj.background_margin;
        if (obj.background_margin_feather) featherPixels.value = obj.background_margin_feather;
        if (obj.background_color) renderBackground.text = Qt.rgba(obj.background_color[0] / 255.0, obj.background_color[1] / 255.0, obj.background_color[2] / 255.0, obj.background_color[3] / 255.0).toString();
    }
    Label {
        position: Label.LeftPosition;
        text: qsTr("Preview resolution");

        ComboBox {
            id: previewResolution;
            model: [QT_TRANSLATE_NOOP("Popup", "Full"), "1080p", "720p", "480p"];
            font.pixelSize: 12 * dpiScale;
            width: parent.width;
            currentIndex: 2;
            onCurrentIndexChanged: {
                let target_height = -1; // Full
                switch (currentIndex) {
                    case 1: target_height = 1080; break;
                    case 2: target_height = 720; break;
                    case 3: target_height = 480; break;
                }

                controller.set_preview_resolution(target_height, window.videoArea.vid);
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
                value: 20;
                defaultValue: 20;
                from: 0;
                to: 50;
                unit: "%";
                precision: 0;
                width: parent.width;
                onValueChanged: controller.background_margin = value / 100;
            }
        }
        Label {
            text: qsTr("Feather");
            SliderWithField {
                id: featherPixels;
                value: 5;
                defaultValue: 5;
                from: 0;
                to: 50;
                unit: "%";
                precision: 0;
                width: parent.width;
                onValueChanged: controller.background_margin_feather = value / 100;
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
            model: [QT_TRANSLATE_NOOP("Popup", "Light"), QT_TRANSLATE_NOOP("Popup", "Dark")];
            font.pixelSize: 12 * dpiScale;
            width: parent.width;
            currentIndex: 1;
            onCurrentIndexChanged: {
                const themes = ["light", "dark"];
                ui_tools.set_theme(themes[currentIndex]);
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
                ["Danish (dansk)",               "da"],
                ["Finnish (suomi)",              "fi"],
                ["French (français)",            "fr"],
                ["Galician (Galego)",            "gl"],
                ["German (Deutsch)",             "de"],
                ["Greek (Ελληνικά)",             "el"],
                ["Indonesian (Bahasa Indonesia)","id"],
                ["Italian (italiano)",           "it"],
                ["Japanese (日本語)",             "ja"],
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
        onCheckedChanged: window.videoArea.safeArea = checked;
    }
    CheckBox {
        //visible: Qt.platform.os != "osx";
        text: qsTr("Experimental zero-copy GPU preview");
        tooltip: qsTr("Render and undistort the preview video entirely on the GPU.\nThis should provide much better UI performance.");
        checked: false;
        onCheckedChanged: controller.set_zero_copy(window.videoArea.vid, checked);
    }
    CheckBox {
        id: gpudecode;
        text: qsTr("Use GPU decoding");
        checked: true;
        onCheckedChanged: controller.set_gpu_decoding(checked);
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
                    processingDevice.preventChange = true;
                    processingDevice.model = [...list, qsTr("CPU only")];
                    for (let i = 0; i < list.length; ++i) {
                        if (list[i] == defaultInitializedDevice) {
                            processingDevice.currentIndex = i;
                            break;
                        }
                    }
                    processingDevice.preventChange = false;
                }
            }
            Component.onCompleted: controller.list_gpu_devices();
            onCurrentIndexChanged: {
                if (preventChange) return;

                if (currentIndex == model.length - 1) {
                    controller.set_device(-1);
                } else {
                    controller.set_device(currentIndex);
                }
            }
        }
    }
}
