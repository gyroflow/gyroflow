// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Window
import QtQuick.Controls.Material
import Qt.labs.settings

import "components/"

Window {
    id: main_window;
    width: 1450;
    height: 800;
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

    Component.onCompleted: ui_tools.set_icon(main_window);

    property bool closeConfirmationModal: false;
    onClosing: (close) => {
        let app = getApp();
        if (app && !closeConfirmationModal) {
            app.messageBox(Modal.NoIcon, qsTr("Are you sure you want to exit?"), [
                { text: qsTr("Yes"), accent: true, clicked: () => main_window.close() },
                { text: qsTr("No"), clicked: () => main_window.closeConfirmationModal = false },
            ]);
            close.accepted = false;
            closeConfirmationModal = true;
        }
    }
    
    App { objectName: "App"; }
}
