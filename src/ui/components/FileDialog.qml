// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC
import QtQuick.Dialogs

import "../Util.js" as Util;

FileDialog {
    id: root;
    property string type: "";
    onAccepted: window.settings.setValue("folder-" + type, filesystem.folder_from_url(selectedFile).toString());

    function open2() {
        const savedFolder = window.settings.value("folder-" + type, "");
        if (savedFolder && Qt.platform.os != "ios") currentFolder = savedFolder;
        open();
    }
}
