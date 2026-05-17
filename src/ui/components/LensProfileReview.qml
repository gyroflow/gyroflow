// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC

import "../menu"

Item {
    id: root;
    property string cameraBrand: "";
    property string cameraModel: "";
    property string lensModel: "";
    property var profiles: [];
    property int currentProfileIndex: 0;
    property var badProfiles: ({}); // checksum -> true

    signal profileSelected(string checksum);
    signal profileMarkedBad(string checksum, bool isBad);

    function loadProfiles(brand: string, model: string, lens: string): void {
        cameraBrand = brand;
        cameraModel = model;
        lensModel = lens;
        // Request profiles from controller
        controller.get_profiles_for_review(brand, model, lens);
    }

    Connections {
        target: controller;
        function onReview_profiles_loaded(profiles_list: var): void {
            root.profiles = profiles_list || [];
            if (root.profiles.length > 0) {
                root.currentProfileIndex = 0;
            }
        }
    }

    Column {
        anchors.fill: parent;
        spacing: 10 * dpiScale;

        Text {
            text: qsTr("Review profiles for: %1 %2 - %3").arg(cameraBrand).arg(cameraModel).arg(lensModel);
            font.pixelSize: 14 * dpiScale;
            font.bold: true;
        }

        Row {
            spacing: 10 * dpiScale;
            width: parent.width;

            Button {
                text: qsTr("Previous");
                enabled: root.currentProfileIndex > 0;
                onClicked: {
                    if (root.currentProfileIndex > 0) {
                        root.currentProfileIndex--;
                        if (root.profiles.length > root.currentProfileIndex) {
                            root.profileSelected(root.profiles[root.currentProfileIndex].checksum || "");
                        }
                    }
                }
            }

            Text {
                text: qsTr("%1 of %2").arg(root.currentProfileIndex + 1).arg(root.profiles.length);
                anchors.verticalCenter: parent.verticalCenter;
            }

            Button {
                text: qsTr("Next");
                enabled: root.currentProfileIndex < root.profiles.length - 1;
                onClicked: {
                    if (root.currentProfileIndex < root.profiles.length - 1) {
                        root.currentProfileIndex++;
                        if (root.profiles.length > root.currentProfileIndex) {
                            root.profileSelected(root.profiles[root.currentProfileIndex].checksum || "");
                        }
                    }
                }
            }
        }

        ScrollView {
            width: parent.width;
            height: parent.height - 200 * dpiScale;

            Column {
                spacing: 10 * dpiScale;
                width: parent.width;

                Repeater {
                    model: root.profiles;
                    Rectangle {
                        width: parent.width;
                        height: profileInfo.height + 20 * dpiScale;
                        color: index === root.currentProfileIndex? styleAccentColor : styleBackground2;
                        border.color: styleTextColor;
                        border.width: 1 * dpiScale;
                        radius: 4 * dpiScale;

                        property var profileData: modelData;

                        Column {
                            id: profileInfo;
                            anchors.left: parent.left;
                            anchors.right: parent.right;
                            anchors.margins: 10 * dpiScale;
                            anchors.verticalCenter: parent.verticalCenter;
                            spacing: 5 * dpiScale;

                            Text {
                                text: profileData.name || qsTr("Profile %1").arg(index + 1);
                                font.pixelSize: 12 * dpiScale;
                                font.bold: true;
                            }

                            Text {
                                text: qsTr("Calibrated by: %1").arg(profileData.calibrated_by || qsTr("Unknown"));
                                font.pixelSize: 11 * dpiScale;
                            }

                            Text {
                                text: qsTr("Rating: %1").arg(profileData.rating || 0);
                                font.pixelSize: 11 * dpiScale;
                            }

                            Text {
                                text: qsTr("RMS Error: %1").arg(profileData.rms || 0);
                                font.pixelSize: 11 * dpiScale;
                                color: (profileData.rms || 0) < 1? "#1ae921" : (profileData.rms || 0) < 5? "#f6a10c" : "#f41717";
                            }

                            Row {
                                spacing: 10 * dpiScale;

                                Button {
                                    text: qsTr("Select");
                                    onClicked: {
                                        root.currentProfileIndex = index;
                                        root.profileSelected(profileData.checksum || "");
                                    }
                                }

                                Button {
                                    text: badProfiles[profileData.checksum]? qsTr("Mark as Good") : qsTr("Mark as Bad");
                                    onClicked: {
                                        const isBad = !badProfiles[profileData.checksum];
                                        if (isBad) {
                                            badProfiles[profileData.checksum] = true;
                                        } else {
                                            delete badProfiles[profileData.checksum];
                                        }
                                        root.profileMarkedBad(profileData.checksum || "", isBad);
                                    }
                                }
                            }
                        }

                        MouseArea {
                            anchors.fill: parent;
                            onClicked: {
                                root.currentProfileIndex = index;
                                root.profileSelected(profileData.checksum || "");
                            }
                        }
                    }
                }
            }
        }
    }
}
