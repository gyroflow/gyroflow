// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

import QtQuick

// Full-size image with a close button, looks like Modal
Rectangle {
    id: root;
    property alias source: img.source;
    property string url: ""; // Opened on a click anywhere on the image outside of `links`
    property var links: []; // [{ rect: [x, y, width, height] in image pixels, url: "" }]
    property bool showDontShowAgain: false;
    property bool opened: false;

    signal closed(bool dontShowAgain);

    function close(): void {
        opened = false;
        closed(dontShowAgain.checked);
        destroy(1000);
    }

    anchors.fill: parent;
    color: "#60000000";
    opacity: pp.opacity;
    visible: opacity > 0;

    MouseArea { visible: root.opened; anchors.fill: parent; preventStealing: true; hoverEnabled: true; onClicked: root.close(); }
    Item {
        id: pp;
        anchors.centerIn: parent;
        anchors.verticalCenterOffset: root.opened? 0 : -50 * dpiScale;
        Ease on anchors.verticalCenterOffset { }
        Ease on opacity { }
        opacity: root.opened? 1 : 0;
        // Image pixels -> UI pixels: natural size, scaled down to fit the window
        property real ratio: Math.min(dpiScale, root.width * 0.9 / Math.max(1, img.implicitWidth), (root.height * 0.9 - footer.height) / Math.max(1, img.implicitHeight));
        width: img.implicitWidth * ratio;
        height: col.height;
        BorderImage {
            anchors.fill: parent;
            anchors.margins: -28;
            border { left: 77; top: 77; right: 77; bottom: 77; }
            horizontalTileMode: BorderImage.Repeat;
            verticalTileMode: BorderImage.Repeat;
            source: "qrc:/resources/shadow.png";
        }
        Column {
            id: col;
            width: parent.width;
            Image {
                id: img;
                width: parent.width;
                height: implicitHeight * pp.ratio;
                smooth: true;
                mipmap: true;
                onStatusChanged: {
                    if (status == Image.Ready) Qt.callLater(() => root.opened = true);
                    if (status == Image.Error) Qt.callLater(() => root.close()); // Later, so the creator has connected to `closed`
                }
                MouseArea {
                    anchors.fill: parent;
                    cursorShape: root.url? Qt.PointingHandCursor : Qt.ArrowCursor;
                    onClicked: if (root.url) Qt.openUrlExternally(root.url);
                }
                Repeater {
                    model: root.links;
                    MouseArea {
                        x: modelData.rect[0] * pp.ratio;
                        y: modelData.rect[1] * pp.ratio;
                        width: modelData.rect[2] * pp.ratio;
                        height: modelData.rect[3] * pp.ratio;
                        cursorShape: Qt.PointingHandCursor;
                        onClicked: Qt.openUrlExternally(modelData.url);
                    }
                }
                Rectangle {
                    anchors.top: parent.top;
                    anchors.right: parent.right;
                    anchors.margins: 8 * dpiScale;
                    width: 30 * dpiScale;
                    height: width;
                    radius: width / 2;
                    color: "#80000000";
                    LinkButton {
                        anchors.fill: parent;
                        iconName: "close";
                        textColor: "#fff";
                        transparent: true;
                        leftPadding: 0; rightPadding: 0; topPadding: 0; bottomPadding: 0;
                        icon.width: 18 * dpiScale;
                        icon.height: 18 * dpiScale;
                        onClicked: root.close();
                    }
                }
            }
            Rectangle {
                id: footer;
                visible: root.showDontShowAgain;
                width: parent.width;
                height: visible? dontShowAgain.height + 10 * dpiScale : 0;
                color: styleBackground2;
                CheckBox {
                    id: dontShowAgain;
                    anchors.centerIn: parent;
                    text: qsTr("Don't show again");
                }
            }
        }
    }
}
