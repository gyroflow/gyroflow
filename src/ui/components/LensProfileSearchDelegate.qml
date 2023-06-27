// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Dialogs
import QtQuick.Controls as QQC
import QtQuick.Controls.impl as QQCI

import "."
import "../menu"

QQC.ItemDelegate {
    id: dlg;
    width: parent? parent.width : 0;
    implicitHeight: 30 * dpiScale;
    property var profilesMenu: null;
    property Popup popup: null;

    property bool isFavorite:  !!profilesMenu.favorites[modelData[2]];
    property bool isOfficial:  modelData[3];
    property bool isPreset:    modelData[1].endsWith(".gyroflow");
    property bool aspectMatch: profilesMenu.currentVideoAspectRatio == 0 || modelData[5] == 0 || profilesMenu.currentVideoAspectRatio == modelData[5] || profilesMenu.currentVideoAspectRatioSwapped == modelData[5];
    property bool hasRating:   rating != 0;
    property real rating:      modelData[4] || 0;

    opacity: aspectMatch? 1.0 : 0.4;

    contentItem: Text {
        anchors.fill: parent;
        text: qsTr(modelData[0]);
        verticalAlignment: Text.AlignVCenter;
        leftPadding: 28 * dpiScale;
        rightPadding: 80 * dpiScale;
        elide: Text.ElideRight;
        color: styleTextColor;
        font: popup.font;
        onImplicitWidthChanged: { if (implicitWidth > popup.maxItemWidth) popup.maxItemWidth = implicitWidth + 80 * dpiScale; }
    }
    Image {
        x: 5 * dpiScale;
        source: isFavorite? "qrc:/resources/icons/svg/star.svg" : "qrc:/resources/icons/svg/star_empty.svg";
        sourceSize.height: 30 * dpiScale * 0.6
        anchors.verticalCenter: parent.verticalCenter;
    }
    scale: dlg.down? 0.99 : 1.0;
    Ease on scale { }
    MouseArea {
        height: parent.height;
        width: 28 * dpiScale;
        cursorShape: Qt.PointingHandCursor;
        onClicked: {
            isFavorite = !isFavorite;
            if (isFavorite) profilesMenu.favorites[modelData[2]] = 1;
            else delete profilesMenu.favorites[modelData[2]];

            profilesMenu.updateFavorites();
        }
    }
    Rectangle {
        visible: isOfficial || isPreset || hasRating;
        anchors.verticalCenter: parent.verticalCenter;
        anchors.right: parent.right;
        anchors.rightMargin: 10 * dpiScale;
        radius: 3 * dpiScale;
        color: isPreset? styleAccentColor : isOfficial? "#64e75a" : (style === "light"? "#e2e2e2" : "#353535");
        width: (isOfficial || isPreset? officialText.width : ratingItem.width) + 8 * dpiScale;
        height: officialText.height + 5 * dpiScale;
        Text {
            id: officialText;
            visible: isOfficial || isPreset;
            text: isPreset? qsTr("preset") : qsTr("official");

            font.pixelSize: 10.5 * dpiScale;
            anchors.centerIn: parent;
        }
        Item {
            id: ratingItem;
            anchors.centerIn: parent;
            visible: !isOfficial && !isPreset && hasRating;
            width: stars1.width;
            height: 10 * dpiScale;
            Image {
                id: stars1;
                source: "qrc:/resources/icons/svg/stars5_empty.svg";
                sourceSize.height: 10 * dpiScale;
            }
            Item {
                clip: true;
                width: stars1.width * (rating / 5);
                height: 10 * dpiScale;
                Image {
                    source: "qrc:/resources/icons/svg/stars5.svg";
                    sourceSize.height: 10 * dpiScale;
                }
            }
        }
    }
    function clickHandler() {
        popup.focus = false;
        popup.parent.focus = true;
        popup.clicked(index);
        popup.visible = false;
    }
    onClicked: clickHandler();
    Keys.onPressed: (e) => {
        if (e.key == Qt.Key_Enter || e.key == Qt.Key_Return) {
            clickHandler();
        }
    }
    background: Rectangle {
        color: dlg.checked? styleAccentColor : (dlg.hovered || dlg.highlighted? styleHighlightColor : "transparent");
        anchors.fill: parent;
        anchors.margins: 2 * dpiScale;
        radius: 4 * dpiScale;
        opacity: dlg.checked? 0.5 : 1.0;
        Rectangle {
            x: 1 * dpiScale;
            color: styleAccentColor;
            height: parent.height * 0.45;
            width: 3 * dpiScale;
            radius: width;
            y: (parent.height - height) / 2;
            visible: popup.lv.currentIndex === index;
        }
    }
    highlighted: popup.highlightedIndex === index;
}
