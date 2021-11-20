import QtQuick 2.15
import MDKVideo

import "components/"
import "menu/" as Menu

Item {
    id: root;
    width: parent.width;
    height: parent.height;
    anchors.horizontalCenter: parent.horizontalCenter;

    property alias vid: vid;
    property alias timeline: timeline;
    property alias durationMs: timeline.durationMs;
    property alias trimStart: timeline.trimStart;
    property alias trimEnd: timeline.trimEnd;
    property alias videoLoader: videoLoader;

    property Menu.VideoInformation vidInfo: null;

    function toLocalFile(u) {
        const s = u.toString();
        return s.substring(s.charAt(9) === ':'? 8 : 7);
    }
    function loadFile(url) {
        if (Qt.platform.os == "android") {
            url = Qt.resolvedUrl("file://" + controller.resolve_android_url(url.toString()));
        }
        vid.loaded = false;
        videoLoader.active = true;
        vidInfo.loader = true;
        //vid.url = url;
        controller.load_video(url, vid);
        const pathParts = toLocalFile(url).split(".");
        pathParts.pop();
        window.outputFile = pathParts.join(".") + "_stabilized.mp4";

        const filename = url.toString().split("/").pop();
        dropText.loadingFile = filename;
        vidInfo.updateEntry("File name", filename);
        vidInfo.updateEntry("Detected camera", "---");
        vidInfo.updateEntry("Contains gyro", "---");
    }
    Connections {
        target: controller;
        function onTelemetry_loaded(is_main_video, filename, camera, imu_orientation, contains_gyro, contains_quats, frame_readout_time) {
            if (is_main_video) {
                vidInfo.updateEntry("Detected camera", camera || "---");
                vidInfo.updateEntry("Contains gyro", contains_gyro? "Yes" : "No");
            }
        }
        function onChart_data_changed() {
            controller.update_chart(timeline.getChart());
        }
    }

    Item {
        width: parent.width;
        height: parent.height - tlcol.height;
        Item {
            property real ratio: vid.videoWidth / Math.max(1, vid.videoHeight);
            property real w: (vid.rotation === 90 || vid.rotation === -90? parent.height : parent.width) - 20 * dpiScale;
            property real h: (vid.rotation === 90 || vid.rotation === -90? parent.width  : parent.height) - 20 * dpiScale;

            width:  (ratio * h) > w? w : (ratio * h);
            height: (w / ratio) > h? h : (w / ratio);
            anchors.centerIn: parent;
            opacity: da.containsDrag? 0.5 : 1.0;

            MDKVideo {
                id: vid;
                visible: opacity > 0;
                opacity: loaded? 1 : 0;
                Ease on opacity { }
                anchors.fill: parent;
                property bool loaded: false;

                onTimestampChanged: {
                    if (!timeline.pressed) {
                        timeline.preventChange = true;
                        timeline.value = timestamp / duration;
                        timeline.preventChange = false;
                    }
                }
                onMetadataLoaded: (md) => {
                    controller.load_telemetry(vid.url, true, vid, timeline.getChart());
                    vidInfo.loadFromVideoMetadata(md);
                    loaded = frameCount > 0;
                    videoLoader.active = false;
                    vidInfo.loader = false;
                    //for (var i in md) console.log(i, md[i]);
                }

                backgroundColor: "#111111";
                Component.onCompleted: {
                    controller.init_player(this);
                }
                Rectangle {
                    border.color: styleVideoBorderColor;
                    border.width: 1 * dpiScale;
                    color: "transparent";
                    radius: 5 * dpiScale;
                    anchors.fill: parent;
                    anchors.margins: -border.width;
                }

                WarningMessage {
                    visible: !controller.lens_loaded;
                    text: qsTr("Lens profile is not loaded, the results will not look correct. Please load a lens profile for your camera."); 
                }
            }
        }
        Rectangle {
            id: dropRect;
            border.width: 3 * dpiScale;
            border.color: style === "light"? Qt.darker(styleBackground, 1.3) : Qt.lighter(styleBackground, 2);
            anchors.fill: parent;
            anchors.margins: 20 * dpiScale;
            anchors.topMargin: 50 * dpiScale;
            anchors.bottomMargin: 50 * dpiScale;
            color: styleBackground;
            radius: 5 * dpiScale;
            opacity: vid.loaded? 0 : da.containsDrag? 0.3 : 1.0;
            Ease on opacity { duration: 300; }
            visible: opacity > 0;
            onVisibleChanged: if (!visible) dropText.loadingFile = "";

            BasicText {
                id: dropText;
                property string loadingFile: "";
                text: loadingFile? qsTr("Loading %1...").arg(loadingFile) : qsTr("Drop video file here");
                font.pixelSize: 30 * dpiScale;
                anchors.centerIn: parent;
                leftPadding: 0;
                scale: dropText.paintedWidth > (parent.width - 50 * dpiScale)? (parent.width - 50 * dpiScale) / dropText.paintedWidth : 1.0;
            }
            DropTargetRect {
                visible: !dropText.loadingFile;
                anchors.fill: dropText;
                anchors.margins: -30 * dpiScale;
                scale: dropText.scale;
            }
            MouseArea {
                visible: !vid.loaded;
                anchors.fill: parent;
                cursorShape: Qt.PointingHandCursor;
                onClicked: vidInfo.selectFileRequest();
            }
        }
        DropArea {
            id: da;
            anchors.fill: dropRect;

            onEntered: (drag) => {
                const ext = drag.urls[0].toString().split(".").pop().toLowerCase();
                drag.accepted = fileDialog.extensions.indexOf(ext) > -1;
            }
            onDropped: (drop) => root.loadFile(drop.urls[0])
        }
        LoaderOverlay {
            id: videoLoader;
            onCancel: controller.cancel_current_operation();
        }

        Connections {
            target: controller;
            function onCompute_progress(id, progress) {
                videoLoader.active = progress < 1;
                videoLoader.cancelable = false;
            }
            function onSync_progress(progress, text) {
                videoLoader.active = progress < 1;
                videoLoader.progress = videoLoader.active? progress : -1;
                videoLoader.text = videoLoader.active? qsTr("Analyzing %1... %2").arg("<b>" + (progress * 100).toFixed(2) + "%</b>").arg("<font size=\"2\">(" + text + ")</font>") : "";
                videoLoader.cancelable = true;
            }
        }
    }

    Column {
        id: tlcol;
        width: parent.width;
        anchors.horizontalCenter: parent.horizontalCenter;
        anchors.bottom: parent.bottom;

        Item {
            width: parent.width;
            height: 40 * dpiScale;

            Row {
                enabled: vid.loaded;
                anchors.verticalCenter: parent.verticalCenter;
                anchors.left: parent.left;
                anchors.leftMargin: 10 * dpiScale;
                height: parent.height;
                BasicText {
                    text: timeline.timeAtPosition((vid.currentFrame + 1) / Math.max(1, vid.frameCount));
                    leftPadding: 0;
                    font.pixelSize: 14 * dpiScale;
                    verticalAlignment: Text.AlignVCenter;
                    height: parent.height;
                }
                BasicText {
                    text: `(${vid.currentFrame+1}/${vid.frameCount})`;
                    leftPadding: 5 * dpiScale;
                    font.pixelSize: 11 * dpiScale;
                    verticalAlignment: Text.AlignVCenter;
                    height: parent.height;
                }
            }

            Row {
                anchors.centerIn: parent;
                spacing: 5 * dpiScale;
                enabled: vid.loaded;
                Button { text: "["; font.bold: true; onClicked: timeline.trimStart = timeline.value; tooltip: qsTr("Trim start"); }
                Button { icon.name: "chevron-left"; tooltip: qsTr("Previous frame"); onClicked: vid.currentFrame -= 1; }
                Button {
                    onClicked: if (vid.playing) vid.pause(); else vid.play();
                    tooltip: vid.playing? qsTr("Pause") : qsTr("Play");
                    icon.name: vid.playing? "pause" : "play";
                }
                Button { icon.name: "chevron-right"; tooltip: qsTr("Next frame"); onClicked: vid.currentFrame += 1; }
                Button { text: "]"; font.bold: true; onClicked: timeline.trimEnd = timeline.value; tooltip: qsTr("Trim end"); }
            }
            Row {
                enabled: vid.loaded;
                spacing: 5 * dpiScale;
                anchors.right: parent.right;
                anchors.rightMargin: 10 * dpiScale;
                anchors.verticalCenter: parent.verticalCenter;
                height: parent.height;

                component SmallLinkButton: LinkButton {
                    height: Math.round(parent.height);
                    anchors.verticalCenter: parent.verticalCenter;
                    textColor: !checked? styleTextColor : styleAccentColor;
                    onClicked: checked = !checked;
                    opacity: checked? 1 : 0.5;
                    checked: true;
                    leftPadding: 6 * dpiScale;
                    rightPadding: 6 * dpiScale;
                    topPadding: 8 * dpiScale;
                    bottomPadding: 8 * dpiScale;
                }

                SmallLinkButton {
                    icon.name: "gyroflow";
                    onCheckedChanged: controller.stab_enabled = checked;
                    tooltip: qsTr("Toggle stabilization");
                }

                SmallLinkButton {
                    icon.name: checked? "sound" : "sound-mute";
                    onClicked: vid.muted = !vid.muted;
                    tooltip: checked? qsTr("Mute") : qsTr("Unmute");
                    checked: !vid.muted;
                }

                ComboBox {
                    model: ["0.13x", "0.25x", "0.5x", "1x", "2x", "4x", "8x", "10x"];
                    width: 60 * dpiScale;
                    currentIndex: 3;
                    height: 25 * dpiScale;
                    itemHeight: 25 * dpiScale;
                    font.pixelSize: 11 * dpiScale;
                    anchors.verticalCenter: parent.verticalCenter;
                    onCurrentTextChanged: {
                        const rate = +currentText.replace("x", ""); // hacky but simple and it works
                        vid.playbackRate = rate;
                    }
                    tooltip: qsTr("Playback speed");
                }
            }
        }

        Item { width: 1; height: 10 * dpiScale; }

        ResizablePanel {
            direction: ResizablePanel.HandleUp;
            implicitHeight: 165 * dpiScale;
            width: parent.width;
            color: "transparent";
            hr.height: 30 * dpiScale;
            additionalHeight: timeline.additionalHeight;
            Timeline {
                id: timeline;
                durationMs: vid.duration;
                anchors.fill: parent;

                onTrimStartChanged: {
                    controller.set_trim_start(trimStart);
                    vid.setPlaybackRange(trimStart * durationMs, trimEnd * durationMs);
                }
                onTrimEndChanged: {
                    controller.set_trim_end(trimEnd);
                    vid.setPlaybackRange(trimStart * durationMs, trimEnd * durationMs);
                }

                property bool preventChange: false;
                onValueChanged: {
                    if (!preventChange) {
                        vid.timestamp = value * vid.duration;
                    }
                }
            }
        }
    }
}
