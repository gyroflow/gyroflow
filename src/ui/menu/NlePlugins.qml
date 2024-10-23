// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Adrian <adrian.eddy at gmail>

import QtQuick
import "../components/"

MenuItem {
    id: root;
    text: qsTr("Video editor plugins");
    iconName: "plugin";
    opened: false;
    objectName: "nlePlugins";

    property string latest_version: "";
    property bool openfx_latest: false;
    property bool adobe_latest: false;
    property string openfx_version: controller.nle_plugins("detect", "openfx");
    property string adobe_version: controller.nle_plugins("detect", "adobe");

    Component.onCompleted: {
        controller.nle_plugins("latest_version", "");
    }
    function selectFolder(type: string, folder: string) {
        const dialog = Qt.createQmlObject("import QtQuick.Dialogs; FolderDialog {}", root, "selectFolderNle");
        dialog.title = qsTr("Select %1").arg(folder);
        const initialFolder = "file://" + folder;
        dialog.currentFolder = initialFolder;
        dialog.accepted.connect(function() {
            if (Qt.resolvedUrl(dialog.selectedFolder) != Qt.resolvedUrl(initialFolder)) {
                root.loader = false;
                messageBox(Modal.Error, qsTr("You selected the wrong folder.\nMake sure to select %1.").arg("<b>" + folder + "</b>"), [ { text: qsTr("Ok"), accent: true } ]);
            } else {
                filesystem.folder_access_granted(dialog.selectedFolder);
                controller.nle_plugins("install", type);
            }
        });
        dialog.rejected.connect(function() {
            root.loader = false;
        });
        dialog.open();
    }

    Connections {
        target: controller;
        function compare_ver(version: string, existing: string): bool {
            const last_part = existing.split(".").pop();
            const is_app_nightly = +version == version;
            if (+last_part > 100) {
                // Nightly plugin is installed
                if (is_app_nightly) {
                    return +last_part >= +version;
                } else {
                    return true;
                }
            } else {
                // Stable plugin is installed
                if (is_app_nightly) {
                    return false;
                } else {
                    return version == existing || version + ".0" == existing;
                }
            }
        }
        function onNle_plugins_result(command: string, result: string) {
            if (command == "latest_version") {
                latest_version = result;
                openfx_latest = openfx_version && compare_ver(latest_version, openfx_version);
                adobe_latest  = adobe_version && compare_ver(latest_version, adobe_version);
            }
            if (command == "install") {
                if (result.startsWith("An error occured")) {
                    messageBox(Modal.Error, result, [ { text: qsTr("Ok"), accent: true } ]);
                }
                openfx_version = controller.nle_plugins("detect", "openfx");
                adobe_version = controller.nle_plugins("detect", "adobe");
                openfx_latest = openfx_version && compare_ver(latest_version, openfx_version);
                adobe_latest  = adobe_version && compare_ver(latest_version, adobe_version);
                root.loader = false;
            }
        }
    }

    Row {
        BasicText {
            text: 'Adobe: <b><font color="%1">%2</font></b> %3'.arg(adobe_version && latest_version? adobe_latest? "#10ee14" : "red" : "").arg(adobe_version? adobe_version : "---").arg(+adobe_version.split(".").pop() > 70? "(nightly)" : "").trim();
            textFormat: Text.StyledText;
            anchors.verticalCenter: parent.verticalCenter;
        }
        LinkButton {
            enabled: !root.loader;
            visible: !adobe_latest;
            text: adobe_version? qsTr("Update") : qsTr("Install");
            leftPadding: 7 * dpiScale;
            rightPadding: 7 * dpiScale;
            onClicked: {
                root.loader = true;
                if (Qt.platform.os == "osx" && isSandboxed) {
                    const folder = "/Library/Application Support/Adobe/Common/Plug-ins/7.0/MediaCore";
                    messageBox(Modal.Info, qsTr("At the next prompt, click <b>\"Open\"</b> to grant access to the %1 folder in order for Gyroflow to install the plugin.").arg("<b>\"" + folder + "\"</b>"), [ { text: qsTr("Ok"), accent: true, clicked: () => {
                        root.selectFolder("adobe", folder);
                    } } ]);
                } else {
                    controller.nle_plugins("install", "adobe");
                }
            }
            anchors.verticalCenter: parent.verticalCenter;
        }
    }

    Row {
        BasicText {
            text: 'OpenFX: <b><font color="%1">%2</font></b> %3'.arg(openfx_version && latest_version? openfx_latest? "#10ee14" : "red" : "").arg(openfx_version? openfx_version : "---").arg(+openfx_version.split(".").pop() > 70? "(nightly)" : "").trim();
            textFormat: Text.StyledText;
            anchors.verticalCenter: parent.verticalCenter;
        }
        LinkButton {
            id: openfxInstall;
            enabled: !root.loader;
            visible: !openfx_latest;
            text: openfx_version? qsTr("Update") : qsTr("Install");
            leftPadding: 7 * dpiScale;
            rightPadding: 7 * dpiScale;
            onClicked: {
                root.loader = true;
                if (Qt.platform.os == "osx" && isSandboxed) {
                    const folder = "/Library/OFX/Plugins";
                    if (!filesystem.exists("file://" + folder)) {
                        const mb = messageBox(Modal.Info, qsTr("%1 folder doesn't exist.\nDue to sandbox limitations, you have to create it yourself.\nOpen <b>Terminal</b> and enter the following command:").arg("<b>\"" + folder + "\"</b>"), [ { text: qsTr("Ok"), accent: true, clicked: () => {
                            openfxInstall.clicked();
                        } }, { text: qsTr("Cancel"), clicked: function() { root.loader = false; } } ]);
                        mb.isWide = true;
                        const tf = Qt.createComponent("../components/TextField.qml").createObject(mb.mainColumn, { readOnly: true });
                        tf.text = "sudo install -m 0755 -o $USER -d /Library/OFX/Plugins";
                        tf.width = mb.mainColumn.width;
                    } else {
                        messageBox(Modal.Info, qsTr("At the next prompt, click <b>\"Open\"</b> to grant access to the %1 folder in order for Gyroflow to install the plugin.").arg("<b>\"" + folder + "\"</b>"), [ { text: qsTr("Ok"), accent: true, clicked: () => {
                            root.selectFolder("openfx", folder);
                        } } ]);
                    }
                } else {
                    controller.nle_plugins("install", "openfx");
                }
            }
            anchors.verticalCenter: parent.verticalCenter;
        }
    }

    LinkButton {
        text: qsTr("More information");
        onClicked: filesystem.open_file_externally("https://github.com/gyroflow/gyroflow-plugins");
        anchors.horizontalCenter: parent.horizontalCenter;
    }
}
