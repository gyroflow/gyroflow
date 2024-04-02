// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC

Rectangle {
    id: root;

    enum MsgType { Info, Warning, Error }
    property int type: InfoMessage.Warning;

    width: parent.width;
    height: t.height + 20 * dpiScale;
    color: type == InfoMessage.Warning? "#f6a10c" :
           type == InfoMessage.Error?   "#f41717" :
           type == InfoMessage.Info?    "#17b6f4" :
           "transparent";
    radius: 5 * dpiScale;
    property alias text: t.text;
    property alias t: t;
    property bool shrinkToText: false;
    function updateSize(): void {;
        if (shrinkToText && tm.contentWidth + 20 * dpiScale < root.parentWidth) {
            t.width = undefined;
            width = Qt.binding(() => tm.contentWidth + 20 * dpiScale);
        } else {
            width = Qt.binding(() => root.parentWidth);
            t.width = Qt.binding(() => root.width - 30 * dpiScale);
        }
    }
    property real parentWidth: parent.width;
    onParentWidthChanged: Qt.callLater(updateSize);
    Ease on opacity { }
    Ease on height { }

    Text {
        id: t;
        font.pixelSize: 13 * dpiScale;
        font.family: styleFont;
        color: type == InfoMessage.Error? "#fff" : "#000";
        width: parent.width - 30 * dpiScale;
        horizontalAlignment: Text.AlignHCenter;
        anchors.centerIn: parent;
        wrapMode: Text.WordWrap;
        onTextChanged: Qt.callLater(root.updateSize);
    }
    Text { // used to calculate width of the text before wrapping
        id: tm;
        visible: false;
        text: t.text;
        font: t.font;
        textFormat: t.textFormat;
    }
}
