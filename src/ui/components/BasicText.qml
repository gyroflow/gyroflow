// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC

Text {
    leftPadding: 10 * dpiScale;
    color: styleTextColor;
    font.pixelSize: 12 * dpiScale;
    font.family: styleFont;
    opacity: enabled? 1.0 : 0.6;
}
