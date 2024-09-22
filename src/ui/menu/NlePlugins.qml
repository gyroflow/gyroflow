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

    Connections {
        target: controller;
        function compare_ver(version: string, existing: string): bool {
            const last_part = existing.split(".").pop();
            const is_app_nightly = +version == version;
            if (+last_part > 70) {
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
                    return version == existing;
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
                openfx_version = controller.nle_plugins("detect", "openfx");
                adobe_version = controller.nle_plugins("detect", "adobe");
                openfx_latest = openfx_version && compare_ver(latest_version, openfx_version);
                adobe_latest  = adobe_version && compare_ver(latest_version, adobe_version);
                root.loader = false;
            }
            console.log("nle_plugins_result", command, result);
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
                controller.nle_plugins("install", "adobe");
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
            enabled: !root.loader;
            visible: !openfx_latest;
            text: openfx_version? qsTr("Update") : qsTr("Install");
            leftPadding: 7 * dpiScale;
            rightPadding: 7 * dpiScale;
            onClicked: {
                root.loader = true;
                controller.nle_plugins("install", "openfx");
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
