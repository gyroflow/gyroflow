import QtQuick 2.15
import QtQuick.Window 2.15
import QtQuick.Controls 2.15 as QQC
import QtQuick.Dialogs

import "."
import "components/"
import "menu/" as Menu

Rectangle {
    id: window;
    visible: true
    color: styleBackground;
    anchors.fill: parent;
    property alias videoArea: videoArea;
    property alias exportSettings: exportSettings;
    property alias outputFile: outputFile.text;
    property alias sync: sync;

    FileDialog {
        id: fileDialog;
        property var extensions: ["mp4", "mov", "mxf", "mkv", "webm"];

        title: qsTr("Choose a video file")
        nameFilters: [qsTr("Video files") + " (*." + extensions.join(" *.") + ")"];
        onAccepted: videoArea.loadFile(fileDialog.selectedFile);
    }

    Row {
        width: parent.width;
        height: parent.height;

        SidePanel {
            id: leftPanel;
            direction: SidePanel.HandleRight;
            topPadding: gflogo.height;
            Item {
                id: gflogo;
                parent: leftPanel;
                width: parent.width;
                height: children[0].height + 35 * dpiScale;
                Image {
                    source: "qrc:/resources/logo" + (style === "dark"? "_white" : "_black") + ".svg"
                    sourceSize.width: parent.width * 0.9;
                    anchors.centerIn: parent;
                }
                Hr { anchors.bottom: parent.bottom; }
            }

            Menu.VideoInformation {
                id: vidInfo;
                onSelectFileRequest: fileDialog.open();
            }
            Menu.LensProfile {
                id: lensProfile;
            }
            Menu.MotionData {
                id: motionData;
            }
        }

        Column {
            width: parent.width - leftPanel.width - rightPanel.width;
            height: parent.height;
            VideoArea {
                id: videoArea;
                height: parent.height - exportbar.height;
                vidInfo: vidInfo;
            }

            // Bottom bar
            Rectangle {
                id: exportbar;
                width: parent.width;
                height: 60 * dpiScale;
                color: styleBackground2;

                Hr { width: parent.width; }

                Row {
                    height: parent.height;
                    spacing: 10 * dpiScale;
                    BasicText {
                        text: qsTr("Output path:");
                        anchors.verticalCenter: parent.verticalCenter;
                    }
                    TextField {
                        id: outputFile;
                        text: "";
                        anchors.verticalCenter: parent.verticalCenter;
                        width: exportbar.width - parent.children[0].width - exportbar.children[2].width - 30 * dpiScale;
                    }
                }

                SplitButton {
                    accent: true;
                    anchors.right: parent.right;
                    anchors.rightMargin: 15 * dpiScale;
                    anchors.verticalCenter: parent.verticalCenter;
                    text: qsTr("Export");
                    icon.name: "video";

                    model: [qsTr("Export .gyroflow file")];

                    onClicked: {
                        controller.render(
                            exportSettings.codec, 
                            outputFile.text, 
                            videoArea.trimStart, 
                            videoArea.trimEnd, 
                            exportSettings.outWidth, 
                            exportSettings.outHeight, 
                            exportSettings.gpu, 
                            exportSettings.audio
                        );
                    }
                    
                    Connections {
                        target: controller;
                        function onRender_progress(progress, frame, total_frames) {
                            videoArea.videoLoader.active = progress < 1;
                            videoArea.videoLoader.progress = videoArea.videoLoader.active? progress : -1;
                            videoArea.videoLoader.text = videoArea.videoLoader.active? qsTr("Rendering %1... %2").arg("<b>" + (progress * 100).toFixed(2) + "%</b>").arg("<font size=\"2\">(" + frame + "/" + total_frames + ")</font>") : "";
                        }
                    }
                }
            }
        }

        SidePanel {
            id: rightPanel;
            direction: SidePanel.HandleLeft;
            Menu.Synchronization {
                id: sync;
            }
            Menu.Stabilization {
                id: stab;
            }
            Menu.Advanced {

            }
            Menu.Export {
                id: exportSettings;
            }
        }
    }

    /*Modal {
        id: modal;
        text: qsTr("Are you sure you want to exit?");
        buttons: [qsTr("Yes"), qsTr("No")];
        accentButton: 0;
        onClicked: (index) => {
            console.log("clicked", index);
            modal.opened = false;
        }
        Component.onCompleted: {
            Qt.callLater(function() {
                modal.opened = true;
            });
        }
    }*/
}
