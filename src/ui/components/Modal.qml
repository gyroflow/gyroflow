// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC
import QtQuick.Controls.impl as QQCI
// import QtQuick.Effects

Rectangle {
    id: root;
    property alias t: t;
    property alias textFormat: t.textFormat;
    property alias text: t.text;
    property alias buttons: btns.model;
    property alias mainColumn: mainColumn;
    property alias mainItem: pp;
    property alias btnsRow: btnsRow;
    property alias dontShowAgain: dontShowAgain;
    property bool opened: false;
    property bool isWide: root.text.length > 200;
    property real widthRatio: 0.8;
    property int accentButton: -1;
    property string modalIdentifier: "";
    default property alias data: mainColumn.data;
    onTextChanged: {
        if (text.indexOf("<") > -1 && text.indexOf("\n") > -1 && textFormat != Text.MarkdownText) {
            text = text.replace(/\n/g, "<br>");
            textFormat = Text.StyledText;
        }
    }

    enum IconType { NoIcon, Info, Warning, Error, Success, Question }

    property int iconType: Modal.Warning;

    signal clicked(int index, bool dontShowAgain);

    function close() {
        opened = false;
        destroy(1000);
    }

    property var loader: null;
    function addLoader(): LoaderOverlay {
        const l = Qt.createComponent("LoaderOverlay.qml").createObject(mainColumn, { cancelable: false, visible: false });
        l.anchors.fill = undefined;
        l.height = Qt.binding(() => l.col.height + 30 * dpiScale);
        l.pb.anchors.verticalCenterOffset = Qt.binding(() => -l.height / 2 + 10 * dpiScale);
        l.width = Qt.binding(() => mainColumn.width);
        root.loader = l;
        return l;
    }

    anchors.fill: parent;
    color: "#60000000";
    opacity: pp.opacity;
    visible: opacity > 0;
    onVisibleChanged: {
        if (visible && iconType != Modal.NoIcon) {
            switch (iconType) {
                case Modal.Info:     icon.iconName = "info";      icon.color = styleAccentColor; break;
                case Modal.Warning:  icon.iconName = "warning";   icon.color = "#f6a10c"; break;
                case Modal.Error:    icon.iconName = "error";     icon.color = "#d82626"; break;
                case Modal.Success:  icon.iconName = "confirmed"; icon.color = "#3cc42f"; break;
                case Modal.Question: icon.iconName = "question";  icon.color = styleAccentColor; break;
            }
            icon.visible = true;
            ease.enabled = false;
            icon.scale = 0.4;
            ease.enabled = true;
            icon.scale = 1;
        }
    }

    MouseArea { visible: root.opened; anchors.fill: parent; preventStealing: true; hoverEnabled: true; onClicked: Qt.inputMethod.hide(); }
    Item {
        id: pp;
        anchors.centerIn: parent;
        anchors.verticalCenterOffset: root.opened? 0 : -50 * dpiScale;
        Ease on anchors.verticalCenterOffset { }
        Ease on opacity { }
        opacity: root.opened? 1 : 0;
        width: Math.min(window.width * 0.95, Math.max(btnsRow.width + 100 * dpiScale, root.isWide? parent.width * (isMobileLayout && !isLandscape? 0.99 : root.widthRatio) : 400 * dpiScale));
        height: col.height;
        property real offs: 0;
        BorderImage {
            anchors.fill: bg;
            anchors.margins: -28;
            border { left: 77; top: 77; right: 77; bottom: 77; }
            horizontalTileMode: BorderImage.Repeat;
            verticalTileMode: BorderImage.Repeat;
            source: "qrc:/resources/shadow.png";
        }
        Rectangle {
            id: bg;
            anchors.fill: parent;
            color: styleBackground2;
            radius: 5 * dpiScale;
            // Replace BorderImage with MultiEffect once Qt is upgraded to 6.6.1 (https://bugreports.qt.io/browse/QTBUG-117830)
            // layer.enabled: true; layer.effect: MultiEffect { shadowEnabled: true; }
        }

        Column {
            id: col;
            width: parent.width;
            anchors.centerIn: parent;
            Item { height: 15 * dpiScale; width: 1; }

            QQCI.IconImage {
                id: icon;
                property string iconName: "";
                visible: false;
                color: styleTextColor;
                height: 70 * dpiScale;
                width: height;
                anchors.horizontalCenter: parent.horizontalCenter;
                source: iconName? "qrc:/resources/icons/svg/" + iconName + ".svg" : "";
                name: iconName;
                layer.enabled: true;
                layer.textureSize: Qt.size(height*2, height*2);
                layer.smooth: true;
                Ease on scale { id: ease; type: Easing.OutElastic; duration: 1000; }
            }

            Item { height: 10 * dpiScale; width: 1; }
            Flickable {
                id: flick;
                width: parent.width;
                height: Math.min(contentHeight, root.height - icon.height - btnsRow.height - 150 * dpiScale);
                boundsBehavior: Flickable.StopAtBounds;
                contentWidth: width;
                contentHeight: mainColumn.height + 25 * dpiScale;
                onWidthChanged: contentWidth = Math.max(t.paintedWidth + 30 * dpiScale, width);
                clip: true;
                QQC.ScrollBar.vertical: QQC.ScrollBar { }
                Column {
                    id: mainColumn;
                    x: 15 * dpiScale;
                    width: parent.width - 2*x;
                    spacing: 10 * dpiScale;
                    BasicText {
                        id: t;
                        width: parent.width;
                        horizontalAlignment: Text.AlignHCenter;
                        wrapMode: Text.WordWrap;
                        font.pixelSize: (root.isWide && screenSize < 7.0? 12 : 14) * dpiScale;
                        rightPadding: 10 * dpiScale;
                        onPaintedWidthChanged: {
                            if (paintedWidth > width) {
                                flick.contentWidth = paintedWidth + 30 * dpiScale;
                            }
                        }

                        MouseArea {
                            anchors.fill: parent;
                            cursorShape: parent.hoveredLink? Qt.PointingHandCursor : Qt.ArrowCursor;
                            acceptedButtons: Qt.NoButton;
                        }
                    }
                }
            }

            Rectangle {
                width: parent.width;
                height: btnsCol.height + 30 * dpiScale;
                color: "#B0" + stylePopupBorder.substring(1);
                radius: 5 * dpiScale;

                Column {
                    id: btnsCol;
                    width: parent.width;
                    anchors.centerIn: parent;
                    onWidthChanged: btnsRow.width = undefined;
                    Flow {
                        id: btnsRow;
                        anchors.horizontalCenter: parent.horizontalCenter;
                        spacing: 10 * dpiScale;
                        function updateWidth() {
                            if (btnsRow.width > btnsCol.width - 20 * dpiScale) {
                                btnsRow.width = btnsCol.width - 20 * dpiScale;
                            }
                        }
                        onWidthChanged: Qt.callLater(btnsRow.updateWidth);
                        Repeater {
                            id: btns;
                            Button {
                                text: modelData;
                                onClicked: root.clicked(index, dontShowAgain.checked);
                                leftPadding: 20 * dpiScale;
                                rightPadding: 20 * dpiScale;
                                height: 32 * dpiScale;
                                accent: root.accentButton === index;
                            }
                        }
                    }
                    Item { visible: root.iconType == Modal.Error; height: 15 * dpiScale; width: 1; }
                    LinkButton {
                        visible: root.iconType == Modal.Error;
                        anchors.horizontalCenter: parent.horizontalCenter;
                        text: qsTr("Troubleshooting");
                        icon.width: 14 * dpiScale;
                        icon.height: 14 * dpiScale;
                        iconName: "external_link";
                        onClicked: Qt.openUrlExternally("https://docs.gyroflow.xyz/app/getting-started/troubleshooting")
                    }
                    Item { visible: root.modalIdentifier; height: 15 * dpiScale; width: 1; }
                    CheckBox {
                        anchors.horizontalCenter: parent.horizontalCenter;
                        x: 20 * dpiScale;
                        id: dontShowAgain;
                        visible: root.modalIdentifier;
                        text: btns.model?.length == 1? qsTr("Don't show again") : qsTr("Remember this choice");
                        checked: false;
                    }
                }
            }
        }
    }
    Shortcut {
        sequence: "Return";
        enabled: root.opened && (root.accentButton > -1 || btns.model?.length == 1);
        onActivated: if (root.opened) root.clicked(btns.model.length > 1? root.accentButton : 0, dontShowAgain.checked);
    }
}
