// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Dialogs as QQD
import "../Util.js" as Util

import "."

TextField {
    id: root;
    text: "";
    width: parent.width;

    property var cbAfterSelect: null;
    property bool folderOnly: false;

    property url fullFileUrl;
    property url folderUrl;
    property string filename;

    property alias outputFileDialog: outputFileDialog;
    property alias outputFolderDialog: outputFolderDialog;

    property bool preventChange: false;

    onTextChanged: {
        // When typing manually
        if (!preventChange) {
            if (Qt.platform.os == "ios" || Qt.platform.os == "android") {
                setFilename(text);
            } else {
                setUrl(filesystem.path_to_url(text));
            }
        }
    }

    function updateText() {
        preventChange = true;
        if (Qt.platform.os == "ios" || Qt.platform.os == "android") {
            text = filename;
        } else {
            text = fullFileUrl.toString()? filesystem.display_url(fullFileUrl) : filesystem.display_folder_filename(folderUrl, filename);
        }
        preventChange = false;
    }

    function setUrl(url: url) {
        fullFileUrl = url;
        filename = filesystem.filename_from_url(url);
        folderUrl = filesystem.folder_from_url(url);
        updateText();
    }
    function setFilename(fname: string) {
        if (fname != filename) {
            filename = fname;
            fullFileUrl = "";
            updateText();
        }
    }
    function setFolder(folder: url) {
        folderUrl = folder;
        if (folder.toString())
            fullFileUrl = "";
        updateText();
    }

    function selectFolder(folder: url, cb) {
        if (folder.toString() && window.allowedOutputUrls.includes(folder.toString())) {
            return cb(folder);
        }
        root.cbAfterSelect = cb;
        outputFolderDialog.open();
    }

    LinkButton {
        anchors.right: parent.right;
        height: parent.height - 1 * dpiScale;
        text: "...";
        font.underline: false;
        font.pixelSize: 15 * dpiScale;
        onClicked: {
            if (Qt.platform.os == "ios" || Qt.platform.os == "android" || root.folderOnly) {
                if (root.folderOnly) {
                    outputFolderDialog.currentFolder = root.folderUrl;
                }
                outputFolderDialog.open();
                return;
            }
            outputFileDialog.defaultSuffix = root.filename.substring(root.filename.length - 3);
            outputFileDialog.selectedFile = root.filename;
            outputFileDialog.currentFolder = root.folderUrl;
            outputFileDialog.open();
        }
    }
    FileDialog {
        id: outputFileDialog;
        fileMode: FileDialog.SaveFile;
        title: qsTr("Select file destination");
        nameFilters: Qt.platform.os == "android"? undefined : [qsTr("Video files") + " (*.mp4 *.mov *.png *.exr)"];
        type: "output-video";
        onAccepted: {
            root.setUrl(outputFileDialog.selectedFile);
            window.exportSettings.updateCodecParams();
        }
    }
    QQD.FolderDialog {
        id: outputFolderDialog;
        title: qsTr("Select file destination");
        onAccepted: {
            root.folderUrl = selectedFolder;
            filesystem.start_accessing_url(root.folderUrl); // This will not have equivalent `stop_accessing_url` because we don't know when the access ends
            updateText();

            window.allowedOutputUrls.push(root.folderUrl.toString());
            if (root.cbAfterSelect) {
                root.cbAfterSelect(root.folderUrl);
                root.cbAfterSelect = null;
            }
        }
    }
}
