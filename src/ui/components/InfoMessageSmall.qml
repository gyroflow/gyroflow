// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC

InfoMessage {
    property bool show: false;
    visible: opacity > 0;
    opacity: show? 1 : 0;
    Ease on opacity { }
    height: (t.height + 10 * dpiScale) * opacity - parent.spacing * (1.0 - opacity);
    t.font.pixelSize: 12 * dpiScale;
    t.x: 5 * dpiScale;
}
