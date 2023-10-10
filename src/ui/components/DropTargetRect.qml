// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick

Canvas {
    id: root;
    property color borderColor: style === "light"? Qt.darker(styleBackground, 1.3) : Qt.lighter(styleBackground, 2);
    anchors.fill: parent;
    contextType: "2d";
    function roundRect(ctx, x: real, y: real, width: real, height: real, r: real) {
        ctx.beginPath();
        ctx.moveTo(x + r, y);
        ctx.lineTo(x + width - r, y);
        ctx.quadraticCurveTo(x + width, y, x + width, y + r);
        ctx.lineTo(x + width, y + height - r);
        ctx.quadraticCurveTo(x + width, y + height, x + width - r, y + height);
        ctx.lineTo(x + r, y + height);
        ctx.quadraticCurveTo(x, y + height, x, y + height - r);
        ctx.lineTo(x, y + r);
        ctx.quadraticCurveTo(x, y, x + r, y);
        ctx.closePath();
    }
    onPaint: {
        const ctx = getContext("2d");
        if (ctx) {
            ctx.setLineDash([2, 5]);
            roundRect(ctx, 5, 5, width - 10, height - 10, 10 * dpiScale);
            ctx.strokeStyle = root.borderColor;
            ctx.lineWidth = 3 * dpiScale;
            ctx.lineCap = "round";
            ctx.stroke();
        }
    }
}