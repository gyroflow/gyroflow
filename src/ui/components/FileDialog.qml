// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC
import QtQuick.Dialogs

import "../Util.js" as Util;

FileDialog {
    id: root;
    property string type: "";
    // In a `Connections` and not in an `onAccepted` handler, because a handler declared where the dialog is used would override one declared here
    Connections {
        target: root;
        function onAccepted(): void { settings.setValue("folder-" + root.type, filesystem.get_folder(root.selectedFile).toString()); }
    }

    function open2(): void {
        const savedFolder = settings.value("folder-" + type, "");
        if (savedFolder) currentFolder = savedFolder;
        open();
    }
}
