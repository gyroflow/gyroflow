import QtQuick 2.15

import "../"
import "../components/"

MenuItem {
    id: root;
    text: qsTr("Video information");
    icon: "info";

    property real videoRotation: 0;

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

        window.exportSettings.orgWidth  = w || 0;
        window.exportSettings.orgHeight = h || 0;
        window.lensProfile.videoWidth   = w || 0;
        window.lensProfile.videoHeight  = h || 0;
        window.exportSettings.outBitrate = +md["stream.video[0].codec.bit_rate"]? ((+md["stream.video[0].codec.bit_rate"] / 1024 / 1024)) : 200;

        list.model["Dimensions"]   = w && h? w + "x" + h : "---";
        list.model["Duration"]     = getDuration(md) || "---";
        list.model["Frame rate"]   = framerate? framerate.toFixed(3) + " fps" : "---";
        list.model["Codec"]        = getCodec(md) || "---";
        list.model["Pixel format"] = getPixelFormat(md) || "---";
        list.model["Rotation"]     = (md["stream.video[0].rotation"] || 0) + " °";
        list.model["Audio"]        = getAudio(md) || "---";
        list.modelChanged();

        root.videoRotation = +(md["stream.video[0].rotation"] || 0);
        // controller.set_video_rotation(-root.videoRotation);
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

    Item {
        id: editRotationControl;
        height: parent.height;
        LinkButton {
            anchors.verticalCenter: parent.verticalCenter;
            icon.name: newRotation.visible? "checkmark" : "pencil";
            icon.height: parent.height * 0.8;
            icon.width: parent.height * 0.8;
            height: newRotation.visible? newRotation.height + 5 * dpiScale : undefined;
            leftPadding: newRotation.visible? 15 * dpiScale : 0; rightPadding: leftPadding;
            x: (newRotation.visible? newRotation.width + 5 * dpiScale : parent.parent.paintedWidth + 15 * dpiScale);
            onClicked: {
                if (newRotation.visible) {
                    newRotation.accepted();
                } else {
                    newRotation.value = root.videoRotation;
                    newRotation.visible = true;
                }
            }
        } 
        NumberField {
            id: newRotation;
            visible: false;
            x: 5 * dpiScale;
            y: (parent.parent.height - height) / 2;
            from: -360;
            to: 360;
            unit: "°";
            height: parent.parent.height + 8 * dpiScale;
            topPadding: 0; bottomPadding: 0;
            width: 50 * dpiScale;
            font.pixelSize: 12 * dpiScale;
            onAccepted: {
                visible = false;
                root.videoRotation = value;
                root.updateEntry("Rotation", root.videoRotation + " °");
                controller.set_video_rotation(root.videoRotation);
            }
        }
    }

    TableList {
        id: list;
        onModelChanged: {
            Qt.callLater(function() {
                if (list) {
                    const rotObj = list.col2.children[list.col2.children.length - 3];
                    editRotationControl.parent = rotObj;
                    editRotationControl.enabled = rotObj.text != '---';
                }
            });
        }
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
