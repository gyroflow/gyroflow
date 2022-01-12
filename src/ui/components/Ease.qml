// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick

Behavior {
    id: ee;
    property int duration: 700;
    property alias type: anim.easing.type;
    NumberAnimation {
        id: anim;
        duration: ee.duration;
        easing.type: Easing.OutExpo;
    }
}