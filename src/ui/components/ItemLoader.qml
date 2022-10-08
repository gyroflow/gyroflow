// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

import QtQuick

Loader {
    asynchronous: true;
    width: parent.width;
    visible: status == Loader.Ready;
    onStatusChanged: if (status === Loader.Ready) window.onItemLoaded(item);
    opacity: visible? 1 : 0;
    Ease on opacity { }
}
