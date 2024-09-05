// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC
import QtQuick.Dialogs

import "../Util.js" as Util;

FileDialog {
    id: root;
    property string type: "";
    onAccepted: settings.setValue("folder-" + type, filesystem.get_folder(selectedFile).toString());

    function open2(): void {
        const savedFolder = settings.value("folder-" + type, "");
        if (savedFolder) currentFolder = savedFolder;
        open();
    }
}
