// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import Qt.labs.settings

import "../components/"

MenuItem {
    text: qsTr("Advanced");
    icon: "settings";
    opened: false;

    Settings {
        id: settings;
        property alias previewResolution: previewResolution.currentIndex;
        property alias renderBackground: renderBackground.text;
        property alias theme: themeList.currentIndex;
        property alias safeAreaGuide: safeAreaGuide.checked;
        property string lang: "en";
    }

    Label {
        position: Label.Left;
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
        position: Label.Left;
        text: qsTr("Render background");

        TextField {
            id: renderBackground;
            text: "#111111";
            width: parent.width;
            onTextChanged: {
                controller.set_background_color(text, window.videoArea.vid);
            }
        }
    }
    Label {
        position: Label.Left;
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
        position: Label.Left;
        text: qsTr("Language");

        ComboBox {
            id: langList;
            property var langs: [
                ["English",           "en"],
                ["Danish (dansk)",    "da"],
                ["German (Deutsch)",  "de"],
                ["Norwegian (norsk)", "no"],
                ["Polish (polski)",   "pl"],
                ["Chinese - Simplified (简体中文)", "zh_CN"],
                ["Chinese - Traditional (繁体中文)", "zh_TW"]
            ];
            Component.onCompleted: {
                let selectedIndex = 0;
                let i = 0;
                model = langs.map((x) => { if (x[1] == settings.lang) { selectedIndex = i; } i++; return x[0]; });
                currentIndex = selectedIndex;
            }
            font.pixelSize: 12 * dpiScale;
            width: parent.width;
            onCurrentIndexChanged: {
                settings.lang = langs[currentIndex][1];
                ui_tools.set_language(settings.lang);
            }
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
        id: zerocopy;
        visible: Qt.platform.os != "osx";
        text: qsTr("Experimental zero-copy GPU preview");
        tooltip: qsTr("Render and undistort the preview video entirely on the GPU.\nThis should provide much better UI performance.");
        checked: false;
        onCheckedChanged: controller.set_zero_copy(window.videoArea.vid, checked);
    }
}
