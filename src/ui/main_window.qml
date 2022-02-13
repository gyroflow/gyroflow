// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Window
import QtQuick.Controls.Material
import Qt.labs.settings

import "components/"

Window {
    id: main_window;
    width: Math.min(Screen.width, 1650 * dpiScale);
    height: Math.min(Screen.height, 950 * dpiScale);
    visible: true;
    color: styleBackground;

    title: "Gyroflow v" + version;
    
    Material.theme: Material.Dark;
    Material.accent: Material.Blue;

    function getApp() {
        for (let i = 0; i < contentItem.children.length; ++i) {
            const x = contentItem.children[i];
            if (x.objectName == "App") return x;
        }
        return null;
    }

    Component.onCompleted: {
        ui_tools.set_icon(main_window);
        Qt.callLater(() => {
            width = width + 1;
            height = height;
        });
    }

    property bool closeConfirmationModal: false;
    property bool closeConfirmed: false;
    onClosing: (close) => {        
        let app = getApp();
        if (app) {
            close.accepted = closeConfirmed || !app.wasModified;
            if (!close.accepted && !closeConfirmationModal) {
                closeConfirmationModal = true;
                app.messageBox(Modal.NoIcon, qsTr("Are you sure you want to exit?"), [
                    { text: qsTr("Yes"), accent: true, clicked: () => {main_window.closeConfirmed = true; main_window.close();} },
                    { text: qsTr("No"), clicked: () => main_window.closeConfirmationModal = false }
                ]);                
            }
        }
    }

    App { objectName: "App"; }
}
