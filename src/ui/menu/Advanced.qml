import QtQuick 2.15
import Qt.labs.settings 1.0

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
                theme.set_theme(themes[currentIndex]);
            }
        }
    }
    Label {
        position: Label.Left;
        text: qsTr("Language");

        ComboBox {
            id: langList;
            property var langs: [
                ["English",         "en"],
                ["Polish - polski", "pl"]
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
                theme.set_language(langs[currentIndex][1]);
                settings.lang = langs[currentIndex][1];
            }
        }
    }
}
