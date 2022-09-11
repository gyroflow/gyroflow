// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Maik Menz

import QtQuick
import Gyroflow

Item {
    id: root;
    property alias graph: graph;
    property alias color: graph.color;
    property alias min: graph.min;
    property alias max: graph.max;
    property alias logY: graph.logY;
    property alias title: title.text;

    BasicText {
        id: title;
        leftPadding: 5 * dpiScale;
    }
    Item {
        width: parent.width;
        y: 18 * dpiScale;
        height: parent.height - y;

        Rectangle {
            color: styleBackground2
            opacity: 0.5;
            anchors.fill: parent;
            radius: 4 * dpiScale;
            border.width: 1;
            border.color: styleVideoBorderColor;
        }
        BasicText {
            anchors.top: parent.top;
            anchors.right: parent.right;
            anchors.topMargin: 5 * dpiScale;
            anchors.rightMargin: 5 * dpiScale;
            text: qsTr("%1 Hz").arg(graph.samplerate/2);
            opacity: 0.5;
        }

        FrequencyGraph {
            id: graph;
            anchors.fill: parent;
            lineWidth: 1.5 * dpiScale;
        }
    }
}