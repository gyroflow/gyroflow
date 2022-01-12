// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC

Column {
    default property alias data: col.data;
    property alias text: cb.text;
    property alias spacing: col.spacing;
    property alias inner: inner;
    property alias checked: cb.checked;

    width: parent.width;
    spacing: 5 * dpiScale;
    property alias cb: cb;
    CheckBox {
        id: cb;
        width: parent.width;
    }
    clip: true;
    Item {
        id: inner;
        x: 10 * dpiScale;
        width: parent.width - x;
        height: cb.checked? col.height + col.y : 0;
        visible: opacity > 0;
        opacity: cb.checked? 1 : 0;

        Ease on opacity { }
        Ease on height { }

        Column {
            y: 5 * dpiScale
            id: col;
            width: parent.width;
            spacing: 5 * dpiScale;
        }
    }
}
