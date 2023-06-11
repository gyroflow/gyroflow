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

    property bool renderAfterSelect: false;
    property bool folderOnly: false;

    property alias outputFileDialog: outputFileDialog;
    property alias outputFolderDialog: outputFolderDialog;

    LinkButton {
        anchors.right: parent.right;
        height: parent.height - 1 * dpiScale;
        text: "...";
        font.underline: false;
        font.pixelSize: 15 * dpiScale;
        onClicked: {
            if (Qt.platform.os == "ios" || root.folderOnly) {
                if (root.folderOnly) {
                    outputFolderDialog.currentFolder = controller.path_to_url(Util.getFolder(root.text));
                }
                outputFolderDialog.open();
                return;
            }
            outputFileDialog.defaultSuffix = root.text.substring(root.text.length - 3);
            outputFileDialog.selectedFile = controller.path_to_url(root.text);
            outputFileDialog.currentFolder = controller.path_to_url(Util.getFolder(root.text));
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
            root.text = controller.url_to_path(outputFileDialog.selectedFile);
            window.exportSettings.updateCodecParams();
        }
    }
    QQD.FolderDialog {
        id: outputFolderDialog;
        title: qsTr("Select file destination");
        property string urlString: "";

        onAccepted: {
            outputFolderDialog.urlString = selectedFolder.toString();
            root.text = controller.url_to_path(selectedFolder) + root.text.split('/').slice(-1);
            window.exportSettings.updateCodecParams();

            if (Qt.platform.os == "ios") {
                controller.start_apple_url_access(outputFolderDialog.urlString);
                // TODO: stop access
                window.allowedOutputUrls.push(outputFolderDialog.urlString);
                if (root.renderAfterSelect) {
                    root.renderAfterSelect = false;
                    window.renderBtn.btn.clicked();
                }
            }
        }
    }
}
