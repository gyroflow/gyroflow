// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

import QtQuick

Loader {
    id: root;
    active: false;
    asynchronous: true;
    property real posx: 0;
    property real posy: 0;
    property Item parentItem;
    onStatusChanged: {
        if (status == Loader.Ready)
            root.item.popup(parentItem, posx, posy);
    }

    function popup(parentItem: Item, x: real, y: real): void {
        root.parentItem = parentItem;
        root.posx = x;
        root.posy = y;
        if (status == Loader.Ready) {
            root.item.popup(parentItem, x, y);
        } else {
            root.active = true;
        }
    }
    function toggle(parentItem: Item, x: real, y: real): void {
        if (root.item && root.item.visible) {
            root.item.close();
        } else {
            root.popup(parentItem, x, y);
        }
    }
}
