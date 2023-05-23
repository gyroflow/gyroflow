// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Maik Menz

import QtQuick
import Gyroflow

import "components/"

Item {
    id: root;

    property bool shown: false;
    opacity: shown? 1 : 0;
    visible: opacity > 0;
    anchors.bottomMargin: (shown? 10 : 30) * dpiScale;
    anchors.topMargin: (shown? 10 : -20) * dpiScale;
    Ease on opacity { }
    Ease on anchors.bottomMargin { }
    Ease on anchors.topMargin { }

    readonly property real timestamp: window.videoArea.vid.timestamp;

    onTimestampChanged: updateCharts();
    onVisibleChanged: updateCharts();

    function updateCharts() {
        if (root.visible) {
            let sr = 800.0;
            switch (samplerate.currentIndex) {
                case 0: sr = 200; break;
                case 1: sr = 400; break;
            }
            const fft_size = 1024;
            controller.update_frequency_graph(gyroX.graph, 0, timestamp, sr, fft_size);
            controller.update_frequency_graph(gyroY.graph, 1, timestamp, sr, fft_size);
            controller.update_frequency_graph(gyroZ.graph, 2, timestamp, sr, fft_size);
            controller.update_frequency_graph(acclX.graph, 3, timestamp, sr, fft_size);
            controller.update_frequency_graph(acclY.graph, 4, timestamp, sr, fft_size);
            controller.update_frequency_graph(acclZ.graph, 5, timestamp, sr, fft_size);
        }
    }

    MouseArea {
        anchors.fill: parent;
        preventStealing: true;
    }

    Rectangle {
        color: styleBackground2
        opacity: 0.8;
        anchors.fill: parent;
        radius: 5 * dpiScale;
        border.width: 1;
        border.color: styleVideoBorderColor;
    }

    BasicText {
        y: 12 * dpiScale;
        x: 5 * dpiScale;
        text: qsTr("Statistics");
        font.pixelSize: 15 * dpiScale;
        font.bold: true;
    }

    LinkButton {
        anchors.right: parent.right;
        width: 34 * dpiScale;
        height: 34 * dpiScale;
        textColor: styleTextColor;
        iconName: "close";
        leftPadding: 0;
        rightPadding: 0;
        topPadding: 10 * dpiScale;
        onClicked: root.shown = false;
    }

    Hr { width: parent.width - 10 * dpiScale; y: 35 * dpiScale; color: "#fff"; opacity: 0.3; }

    Row {
        spacing: 10 * dpiScale;
        x: spacing;
        y: 36 * dpiScale + spacing;
        width: parent.width - 2 * spacing;
        height: parent.height - y - spacing - 30 * dpiScale;

        Column {
            width: (parent.width + parent.spacing) / 2 - parent.spacing;
            height: parent.height;
            spacing: parent.spacing;

            readonly property real itemHeight: (height + spacing) / children.length - spacing;

            FrequencyChart {
                id: gyroX;
                title: qsTr("Gyro-X");
                width: parent.width;
                height: parent.itemHeight;
                color: "#8f4c4c";
                logY: logValue.checked;
                min: 0.001;
                max: 1.0;
            }
            FrequencyChart {
                id: gyroY;
                title: qsTr("Gyro-Y");
                width: parent.width;
                height: parent.itemHeight;
                color: "#4c8f4d";
                logY: logValue.checked;
                min: 0.001;
                max: 1.0;
            }
            FrequencyChart {
                id: gyroZ;
                title: qsTr("Gyro-Z");
                width: parent.width;
                height: parent.itemHeight;
                color: "#4c7c8f";
                logY: logValue.checked;
                min: 0.001;
                max: 1.0;
            }
        }

        Column {
            width: (parent.width + parent.spacing) / 2 - parent.spacing;
            height: parent.height;
            spacing: parent.spacing;

            readonly property real itemHeight: (height + spacing) / children.length - spacing;

            FrequencyChart {
                id: acclX;
                title: qsTr("Accl-X");
                width: parent.width;
                height: parent.itemHeight;
                color: "#8f4c4c";
                logY: logValue.checked;
                min: 0.001;
                max: 1.0;
            }
            FrequencyChart {
                id: acclY;
                title: qsTr("Accl-Y");
                width: parent.width;
                height: parent.itemHeight;
                color: "#4c8f4d";
                logY: logValue.checked;
                min: 0.001;
                max: 1.0;
            }
            FrequencyChart {
                id: acclZ;
                title: qsTr("Accl-Z");
                width: parent.width;
                height: parent.itemHeight;
                color: "#4c7c8f";
                logY: logValue.checked;
                min: 0.001;
                max: 1.0;
            }
        }
    }

    CheckBox {
        id: logValue;

        anchors.left: parent.left;
        anchors.bottom: parent.bottom;
        anchors.leftMargin: 10 * dpiScale;
        anchors.bottomMargin: 5 * dpiScale;

        text: qsTr("Logarithmic value axis");
        checked: true;
    }
    Label {
        width: 200 * dpiScale;
        position: Label.LeftPosition;
        text: qsTr("Sample rate");

        anchors.right: parent.right;
        anchors.bottom: parent.bottom;
        anchors.rightMargin: 10 * dpiScale;
        anchors.bottomMargin: 5 * dpiScale;

        ComboBox {
            id: samplerate;
            model: ["200 Hz", "400 Hz", "800 Hz"];
            font.pixelSize: 12 * dpiScale;

            width: parent.width;
            height: 25 * dpiScale;

            currentIndex: 1;
            onCurrentIndexChanged: root.updateCharts();
            Component.onCompleted: currentIndexChanged();
        }
    }
}