// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC
import QtQuick.Dialogs

import "../Util.js" as Util;

FileDialog {
    id: root;
    property string type: "";
    onAccepted: window.settings.setValue("folder-" + type, Util.getFolder(controller.url_to_path(selectedFile)));

    function open2() {
        const savedFolder = window.settings.value("folder-" + type, "");
        if (savedFolder && Qt.platform.os != "ios") currentFolder = controller.path_to_url(savedFolder);
        open();
    }
}
