// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick

import "../"
import "../components/"

MenuItem {
    id: root;
    text: qsTr("Video information");
    icon: "info";

    property real videoRotation: 0;
    property real fps: 0;
    property real org_fps: 0;
    property string filename: "";
    property bool isCalibrator: false;
    property string pixelFormat: "";

    Component.onCompleted: {
        const fields = [
            QT_TRANSLATE_NOOP("TableList", "File name"),
            QT_TRANSLATE_NOOP("TableList", "Detected camera"),
            QT_TRANSLATE_NOOP("TableList", "Dimensions"),
            QT_TRANSLATE_NOOP("TableList", "Duration"),
            QT_TRANSLATE_NOOP("TableList", "Frame rate"),
            QT_TRANSLATE_NOOP("TableList", "Codec"),
            QT_TRANSLATE_NOOP("TableList", "Pixel format"),
            QT_TRANSLATE_NOOP("TableList", "Audio"),
            QT_TRANSLATE_NOOP("TableList", "Rotation"),
            QT_TRANSLATE_NOOP("TableList", "Contains gyro")
        ];
        let model = {};
        for (const x of fields) model[x] = "---";
        list.model = model;
    }

    signal selectFileRequest();

    function loadFromVideoMetadata(md) {
        const framerate = +md["stream.video[0].codec.frame_rate"];
        const w = md["stream.video[0].codec.width"];
        const h = md["stream.video[0].codec.height"];

        if (window) {
            window.exportSettings.orgWidth  = w || 0;
            window.exportSettings.orgHeight = h || 0;
            window.lensProfile.videoWidth   = w || 0;
            window.lensProfile.videoHeight  = h || 0;
            window.exportSettings.outBitrate = +md["stream.video[0].codec.bit_rate"]? ((+md["stream.video[0].codec.bit_rate"] / 1024 / 1024)) : 200;
        }
        if (typeof calibrator_window !== "undefined") {
            calibrator_window.lensCalib.videoWidth   = w || 0;
            calibrator_window.lensCalib.videoHeight  = h || 0;
        }

        root.pixelFormat = getPixelFormat(md) || "---";

        list.model["Dimensions"]   = w && h? w + "x" + h : "---";
        list.model["Duration"]     = getDuration(md) || "---";
        list.model["Frame rate"]   = framerate? framerate.toFixed(3) + " fps" : "---";
        list.model["Codec"]        = getCodec(md) || "---";
        list.model["Pixel format"] = root.pixelFormat;
        list.model["Rotation"]     = (md["stream.video[0].rotation"] || 0) + " °";
        list.model["Audio"]        = getAudio(md) || "---";
        list.modelChanged();

        root.videoRotation = +(md["stream.video[0].rotation"] || 0);
        root.fps = framerate;
        root.org_fps = framerate;
        // controller.set_video_rotation(-root.videoRotation);
    }
    function updateEntry(key, value) {
        if (key == "File name") root.filename = value;
        list.updateEntry(key, value);
    }

    function getDuration(md) {
        const s = +md["stream.video[0].duration"] / 1000;
        if (s > 60) {
            return Math.floor(s / 60) + " m " + Math.floor(s % 60) + " s";
        } else if (s > 0) {
            return s.toFixed(2) + " s";
        }
        return "";
    }
    function getCodec(md) {
        const c = md["stream.video[0].codec.name"] || "";
        const bitrate = +md["stream.video[0].codec.bit_rate"]? ((+md["stream.video[0].codec.bit_rate"] / 1024 / 1024).toFixed(2) + " Mbps") : "";

        return c.toUpperCase() + (c? " " : "") + bitrate;
    }
    function getPixelFormat(md) {
        let pt = md["stream.video[0].codec.format_name"] || "";
        let bits = "8 bit";
        if (pt.indexOf("p10le") > -1) { bits = "10 bit"; pt = pt.replace("p10le", ""); } // TODO detect more formats

        return pt.toUpperCase() + (pt? " " : "") + bits;
    }
    function getAudio(md) {
        const format = md["stream.audio[0].codec.name"]? (md["stream.audio[0].codec.name"].replace("_", " ").replace("pcm", "PCM").replace("aac", "AAC")) : "";
        const rate = md["stream.audio[0].codec.sample_rate"]? (md["stream.audio[0].codec.sample_rate"] + " Hz") : "";

        return format + (format? " " : "") + rate;
    }

    Button {
        text: qsTr("Open file");
        icon.name: "video"
        anchors.horizontalCenter: parent.horizontalCenter;
        onClicked: root.selectFileRequest();
    }

    TableList {
        id: list;
        spacing: 6 * dpiScale;
        editableFields: isCalibrator? ({}) : ({
            "Rotation": {
                "unit": "°",
                "from": -360,
                "to": 360, 
                "value": function() { return root.videoRotation; },
                "onChange": function(value) {
                    root.videoRotation = value;
                    root.updateEntry("Rotation", root.videoRotation + " °");
                    controller.set_video_rotation(root.videoRotation);
                }
            },
            "Frame rate": {
                "unit": "fps",
                "precision": 3,
                "width": 70,
                "value": function() { return root.fps; },
                "onChange": function(value) {
                    root.fps = value;
                    root.updateEntry("Frame rate", value.toFixed(3) + " fps");
                    controller.override_video_fps(value);

                    const scale = root.fps / root.org_fps;
                    window.sync.everyNthFrame = Math.max(1, Math.floor(scale));

                    const chart = window.videoArea.timeline.getChart();
                    chart.setDurationMs(controller.get_scaled_duration_ms());
                    window.videoArea.durationMs = controller.get_scaled_duration_ms();
                    Qt.callLater(() => controller.update_chart(window.videoArea.timeline.getChart())); 
                }
            }
        });
    }

    DropTarget {
        parent: root.innerItem;
        z: 999;
        anchors.rightMargin: -28 * dpiScale;
        anchors.topMargin: 35 * dpiScale;
        anchors.bottomMargin: -35 * dpiScale;
        extensions: fileDialog.extensions;
        onLoadFile: (path) => window.videoArea.loadFile(path)
    }
}
