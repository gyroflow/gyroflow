import QtQuick 2.15

import "../"
import "../components/"

MenuItem {
    id: root;
    text: qsTr("Video information");
    icon: "info";

    Component.onCompleted: {
        const fields = [
            QT_TR_NOOP("File name"),
            QT_TR_NOOP("Detected camera"),
            QT_TR_NOOP("Dimensions"),
            QT_TR_NOOP("Duration"),
            QT_TR_NOOP("Frame rate"),
            QT_TR_NOOP("Codec"),
            QT_TR_NOOP("Pixel format"),
            QT_TR_NOOP("Audio"),
            QT_TR_NOOP("Rotation"),
            QT_TR_NOOP("Contains gyro")
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

        window.exportSettings.orgWidth = w;
        window.exportSettings.orgHeight = h;
        window.exportSettings.bitrate = +md["stream.video[0].codec.bit_rate"]? ((+md["stream.video[0].codec.bit_rate"] / 1024 / 1024)) : 200;

        list.model["Dimensions"]   = w && h? w + "x" + h : "---";
        list.model["Duration"]     = getDuration(md) || "---";
        list.model["Frame rate"]   = framerate? framerate.toFixed(3) + " fps" : "---";
        list.model["Codec"]        = getCodec(md) || "---";
        list.model["Pixel format"] = getPixelFormat(md) || "---";
        list.model["Rotation"]     = (md["stream.video[0].rotation"] || 0) + " °";
        list.model["Audio"]        = getAudio(md) || "---";
        list.modelChanged();
    }
    function updateEntry(key, value) {
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
    TableList { id: list; }

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
