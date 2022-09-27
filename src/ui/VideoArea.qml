// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import MDKVideo

import "components/"
import "menu/" as Menu
import "Util.js" as Util

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
    property alias stabEnabledBtn: stabEnabledBtn;
    property alias queue: queue;
    property alias statistics: statistics;

    property int outWidth: window? window.exportSettings.outWidth : 0;
    property int outHeight: window? window.exportSettings.outHeight : 0;

    property alias dropRect: dropRect;
    property bool isCalibrator: false;

    property bool safeArea: false;
    property var pendingGyroflowData: null;
    property url loadedFileUrl;

    property bool fullScreen: false;

    property Menu.VideoInformation vidInfo: null;

    function loadGyroflowData(obj) {
        root.pendingGyroflowData = null;

        if (controller.loading_gyro_in_progress) {
            root.pendingGyroflowData = obj;
            controller.cancel_current_operation();
            // we'll get called again from telemetry_loaded
            return;
        }

        let paths = null;

        if (obj.toString().startsWith("file")) { // obj is url
            paths = controller.get_paths_from_gyroflow_file(obj);
        } else {
            paths = [
                obj.videofile,
                obj.gyro_source?.filepath || ""
            ];
        }

        const isCorrectVideoLoaded = paths[0] && vidInfo.filename == Util.getFilename(paths[0]);
        const isCorrectGyroLoaded  = paths[1] && window.motionData.filename == Util.getFilename(paths[1]);
        console.log("Video path:", paths[0], "(" + (isCorrectVideoLoaded? "loaded" : "not loaded") + ")", "Gyro path:", paths[1], "(" + (isCorrectGyroLoaded? "loaded" : "not loaded") + ")");

        if (paths[0] && !isCorrectVideoLoaded) {
            root.pendingGyroflowData = obj;
            console.log("Loading video file", paths[0]);
            loadFile(controller.path_to_url(paths[0]), false);
            if (controller.image_sequence_fps > 0) {
                vid.setFrameRate(controller.image_sequence_fps);
            }
            return;
        }
        if (paths[1] && !isCorrectGyroLoaded && controller.file_exists(paths[1])) {
            root.pendingGyroflowData = obj;
            console.log("Loading gyro file", paths[1]);
            controller.load_telemetry(controller.path_to_url(paths[1]), paths[0] == paths[1], window.videoArea.vid, window.videoArea.timeline.getChart(), window.videoArea.timeline.getKeyframesView());
            return;
        }

        if (obj.toString().startsWith("file")) {
            // obj is url
            controller.import_gyroflow_file(obj);
        } else {
            controller.import_gyroflow_data(JSON.stringify(obj));
        }
    }
    Connections {
        target: controller;
        function onGyroflow_file_loaded(obj) {
            if (obj && +obj.version > 0) {
                const info = obj.video_info || { };
                if (info && Object.keys(info).length > 0) {
                    if (info.hasOwnProperty("vfr_fps") && Math.round(+info.vfr_fps * 1000) != Math.round(+info.fps * 1000)) {
                        vidInfo.updateEntryWithTrigger("Frame rate", +info.vfr_fps);
                    }
                    if (info.hasOwnProperty("rotation") && Math.abs(+info.rotation) > 0) {
                        vidInfo.updateEntryWithTrigger("Rotation", +info.rotation);
                    }
                }

                for (const ts in obj.offsets) {
                    controller.set_offset(ts, obj.offsets[ts]);
                }
                if (obj.hasOwnProperty("trim_start")) {
                    timeline.setTrim(obj.trim_start, obj.trim_end);
                }
                window.motionData.loadGyroflow(obj);
                window.stab.loadGyroflow(obj);
                window.advanced.loadGyroflow(obj);
                window.sync.loadGyroflow(obj);
                Qt.callLater(window.exportSettings.loadGyroflow, obj);

                if (obj.hasOwnProperty("image_sequence_start") && +obj.image_sequence_start > 0) {
                    controller.image_sequence_start = +obj.image_sequence_start;
                }
                if (obj.hasOwnProperty("image_sequence_fps") && +obj.image_sequence_fps > 0.0) {
                    vid.setFrameRate(+obj.image_sequence_fps);
                    controller.image_sequence_fps = +obj.image_sequence_fps;
                }
                if (obj.hasOwnProperty("playback_speed")) {
                    let i = 0;
                    const speed = +obj.playback_speed;
                    for (const x of playbackRateCb.model) {
                        const rate = +x.replace("x", "");
                        if (Math.abs(rate - speed) < 0.01) {
                            playbackRateCb.currentIndex = i;
                            break;
                        }
                        ++i;
                    }
                }
                if (obj.hasOwnProperty("muted")) {
                    videoArea.vid.muted = !!obj.muted;
                }
            }
        }
        function onExternal_sdk_progress(percent: real, sdk_name: string, error_string: string, path: string) {
            if (externalSdkModal !== null && externalSdkModal.loader !== null) {
                externalSdkModal.loader.visible = percent < 1;
                externalSdkModal.loader.active = percent < 1;
                externalSdkModal.loader.progress = percent;
                externalSdkModal.loader.text = qsTr("Downloading %1 (%2)").arg(sdk_name);
                if (percent >= 1) {
                    externalSdkModal.close();
                    externalSdkModal = null;
                    window.isDialogOpened = false;
                    if (!error_string) {
                        if (path == "ffmpeg_gpl") {
                            messageBox(Modal.Success, qsTr("Component was installed successfully.\nYou need to restart Gyroflow for changes to take effect.\nYour render queue and current file is saved automatically."), [ { text: qsTr("Ok") } ]);
                        } else {
                            loadFile(path, false);
                        }
                    } else {
                        messageBox(Modal.Error, error_string, [ { text: qsTr("Ok") } ]);
                    }
                }
            }
        }
        function onMp4_merge_progress(percent: real, error_string: string, path: string) {
            if (externalSdkModal !== null && externalSdkModal.loader !== null) {
                externalSdkModal.loader.visible = percent < 1;
                externalSdkModal.loader.active = percent < 1;
                externalSdkModal.loader.progress = percent;
                externalSdkModal.loader.text = qsTr("Merging files to %1 (%2)").arg("<b>" + path + "</b>");
                if (percent >= 1) {
                    externalSdkModal.close();
                    externalSdkModal = null;
                    window.isDialogOpened = false;
                    if (!error_string) {
                        loadFile(controller.path_to_url(path), true);
                    } else {
                        messageBox(Modal.Error, error_string, [ { text: qsTr("Ok") } ]);
                    }
                }
            }
        }
    }
    property Modal externalSdkModal: null;

    function loadFile(url: url, skip_detection: bool) {
        if (Qt.platform.os == "android") {
            url = Qt.resolvedUrl("file://" + controller.resolve_android_url(url.toString()));
        }

        if (url.toString().endsWith(".gyroflow")) {
            return loadGyroflowData(url);
        }

        if (controller.check_external_sdk(url.toString())) {
            const dlg = messageBox(Modal.Info, qsTr("This format requires an external SDK. Do you want to download it now?"), [
                { text: qsTr("Yes"), accent: true, clicked: function() {
                    dlg.btnsRow.children[0].enabled = false;
                    controller.install_external_sdk(url.toString());
                    return false;
                } },
                { text: qsTr("Cancel"), clicked: function() {
                    externalSdkModal = null;
                } },
            ]);
            externalSdkModal = dlg;
            dlg.addLoader();
            return;
        }

        root.loadedFileUrl = url;
        if (!skip_detection) {
            let newUrl;
            if (newUrl = detectImageSequence(url)) {
                const dlg = messageBox(Modal.Info, qsTr("Image sequence has been detected.\nPlease provide frame rate: "), [
                    { text: qsTr("Ok"), accent: true, clicked: function() {
                        const fps = dlg.mainColumn.children[1].value;
                        controller.image_sequence_fps = fps;
                        loadFile(newUrl, true);
                        vid.setFrameRate(fps);
                    } },
                    { text: qsTr("Cancel") },
                ]);
                const nf = Qt.createComponent("components/NumberField.qml").createObject(dlg.mainColumn, { precision: 3, unit: "fps", value: 30.0 });
                nf.anchors.horizontalCenter = dlg.mainColumn.horizontalCenter;
                return;
            }
            let sequenceList;
            if (sequenceList = detectVideoSequence(url)) {
                const list = "<b>" + sequenceList.map(x => x.split('/').pop()).join(", ") + "</b>";
                const dlg = messageBox(Modal.Info, qsTr("Split recording has been detected, do you want to automatically join the files (%1) to create one full clip?").arg(list), [
                    { text: qsTr("Yes"), accent: true, clicked: function() {
                        dlg.btnsRow.children[0].enabled = false;
                        controller.mp4_merge(sequenceList);
                        return false;
                    } },
                    { text: qsTr("No"), clicked: function() {
                        externalSdkModal = null;
                        loadFile(url, true);
                    } },
                ])
                externalSdkModal = dlg;
                dlg.addLoader();
                return;
            }
        }
        window.stab.fovSlider.value = 1.0;
        vid.loaded = false;
        videoLoader.active = true;
        vidInfo.loader = true;
        //vid.url = url;
        vid.errorShown = false;
        render_queue.editing_job_id = 0;
        controller.load_video(url, vid);
        const pathParts = url.toString().split(".");
        pathParts.pop();
        if (!isCalibrator) {
            const suffix = window.advanced.defaultSuffix.text;
            window.outputFile = controller.url_to_path(pathParts.join(".") + suffix + ".mp4").replace(/%0[0-9]+d/, "");
            window.exportSettings.updateCodecParams();
        }
        if (!root.pendingGyroflowData) {
            const gfUrl = pathParts.join(".") + ".gyroflow";
            const gfFile = controller.url_to_path(gfUrl);
            if (controller.file_exists(gfFile)) {
                const gfFilename = gfFile.replace(/\\/g, "/").split("/").pop();
                messageBox(Modal.Question, qsTr("There's a %1 file associated with this video, do you want to load it?").arg("<b>" + gfFilename + "</b>"), [
                    { text: qsTr("Yes"), clicked: function() {
                        Qt.callLater(() => loadFile(gfUrl, true));
                    } },
                    { text: qsTr("No"), accent: true },
                ]);
            }
        }

        const filename = controller.url_to_path(url).split("/").pop();
        dropText.loadingFile = filename;
        vidInfo.updateEntry("File name", filename);
        vidInfo.updateEntry("Detected camera", "---");
        vidInfo.updateEntry("Contains gyro", "---");
        timeline.editingSyncPoint = false;
    }
    function loadMultipleFiles(urls: list, skip_detection: bool) {
        if (urls.length == 1) {
            root.loadFile(urls[0], skip_detection);
        } else if (urls.length > 1) {
            const paths = urls.map(x => controller.url_to_path(x));
            const dlg = messageBox(Modal.Question, qsTr("You have opened multiple files. What do you want to do?"), [
                { text: qsTr("Add to render queue"), clicked: function() {
                    queue.dt.loadFiles(urls);
                    queue.shown = true;
                } },
                { text: qsTr("Merge them into one video"), clicked: function() {
                    dlg.btnsRow.children[0].enabled = false;
                    dlg.btnsRow.children[1].enabled = false;
                    dlg.btnsRow.children[2].enabled = false;
                    controller.mp4_merge(paths);
                    return false;
                } },
                { text: qsTr("Open the first file"), clicked: function() {
                    root.loadFile(urls[0], skip_detection);
                } },
                { text: qsTr("Cancel") },
            ]);
            externalSdkModal = dlg;
            dlg.addLoader();
        }
    }
    function detectImageSequence(url: url) {
        const urlStr = controller.url_to_path(url);
        if (!urlStr.includes("%0")) {
            controller.image_sequence_start = 0;
            controller.image_sequence_fps = 0;
        }
        if (/\d+\.(png|exr|dng)$/i.test(urlStr)) {
            let firstNum = urlStr.match(/(\d+)\.(png|exr|dng)$/i);
            if (firstNum[1]) {
                const ext = firstNum[2];
                firstNum = firstNum[1];
                const firstNumNum = parseInt(firstNum, 10);
                for (let i = firstNumNum + 1; i < firstNumNum + 5; ++i) { // At least 5 frames
                    const newNum = i.toString().padStart(firstNum.length, '0');
                    const newPath = urlStr.replace(firstNum + "." + ext, newNum + "." + ext);
                    if (!controller.file_exists(newPath)) {
                        return false;
                    }
                }
                controller.image_sequence_start = firstNumNum;
                return controller.path_to_url(urlStr.replace(`${firstNum}.${ext}`, `%0${firstNum.length}d.${ext}`));
            }
        }
        return false;
    }
    function detectVideoSequence(url: url) {
        const urlStr = controller.url_to_path(url);

        // url pattern, new path function, additional condition
        const patterns = [
            // GoPro
            [/(G[XH](01).+\.MP4)$/i, function(match, i) {
                return match.substring(0, 2) + i.toString().padStart(2, '0') + match.substring(4);
            }],
            // DJI Action
            [/(DJI_\d+_(\d+)\.MP4)$/i, function(match, i) {
                return match.substring(0, 9) + i.toString().padStart(3, '0') + match.substring(12);
            }],
            // DJI, by duration
            [/(DJI_(\d+)\.MP4)$/i, function(match, i) {
                return match.substring(0, 4) + i.toString().padStart(4, '0') + ".MP4";
            }, function(newPath, list) {
                // DJI splits the files after 6 minutes
                return !list.length || (controller.video_duration(list[list.length - 1]) > 358 && controller.video_duration(newPath) < 358)
            }],
        ];
        for (const x of patterns) {
            let match = urlStr.match(x[0]);
            if (match && match[1]) {
                let list = [];
                const firstNum = parseInt(match[2], 10);
                for (let i = firstNum; i < firstNum + 20; ++i) { // Try 20 parts
                    const newPath = urlStr.replace(match[1], x[1](match[1], i));
                    if (controller.file_exists(newPath) && (x[2]? x[2](newPath, list) : true)) {
                        list.push(newPath);
                    } else {
                        break;
                    }
                }
                if (list.length > 1)
                    return list;
            }
        }
        return false;
    }

    Connections {
        target: controller;
        function onTelemetry_loaded(is_main_video: bool, filename: string, camera: string, imu_orientation: string, contains_gyro: bool, contains_raw_gyro: bool, contains_quats: bool, frame_readout_time: real, camera_id_json: string, sample_rate: real) {
            if (is_main_video) {
                vidInfo.updateEntry("Detected camera", camera || "---");
                vidInfo.updateEntry("Contains gyro", contains_gyro? "Yes" : "No");
                // If source was detected, but gyro data is empty
                if (camera) {
                    if (!contains_gyro && !contains_quats) {
                        messageBox(Modal.Warning, qsTr("File format was detected, but no motion data was found.\nThe camera probably doesn't record motion data in this particular shooting mode."), [ { "text": qsTr("Ok") } ]);
                    }
                    if (contains_raw_gyro && !contains_quats) timeline.setDisplayMode(0); // Switch to gyro view
                    if (!contains_raw_gyro && contains_quats) timeline.setDisplayMode(3); // Switch to quaternions view
                }
            }
            if (sample_rate > 0.0 && sample_rate < 50) {
                messageBox(Modal.Warning, qsTr("Motion data sampling rate is too low (%1 Hz).\n50 Hz is an absolute minimum and we recommend at least 200 Hz.").arg(sample_rate.toFixed(0)), [ { "text": qsTr("Ok") } ]);
            }
            if (root.pendingGyroflowData) {
                Qt.callLater(loadGyroflowData, root.pendingGyroflowData);
            } else {
                Qt.callLater(controller.recompute_threaded);
            }
        }
        function onChart_data_changed() {
            chartUpdateTimer.start();
        }
        function onKeyframes_changed() {
            Qt.callLater(controller.update_keyframes_view, timeline.getKeyframesView());
            Qt.callLater(controller.update_keyframe_values, vid.timestamp);
        }
    }
    Timer {
        id: chartUpdateTimer;
        repeat: false;
        running: false;
        interval: 100;
        onTriggered: Qt.callLater(controller.update_chart, timeline.getChart());
    }

    Item {
        width: parent.width;
        height: parent.height - (root.fullScreen? 0 : tlcol.height);
        Item {
            id: vidParent;
            property real orgW: root.outWidth || vid.videoWidth;
            property real orgH: root.outHeight || vid.videoHeight;
            property real ratio: orgW / Math.max(1, orgH);
            property real w: parent.width - 20 * dpiScale;
            property real h: parent.height - 20 * dpiScale;

            width:  (ratio * h) > w? w : (ratio * h);
            height: (w / ratio) > h? h : (w / ratio);
            anchors.centerIn: parent;
            opacity: da.containsDrag? 0.5 : 1.0;
            clip: !vid.stabEnabled;

            /*Image {
                // Transparency grid
                fillMode: Image.Tile;
                anchors.fill: parent;
                source: "data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='14' height='14'><rect fill='%23fff' x='0' y='0' width='7' height='7'/><rect fill='%23aaa' x='7' y='0' width='7' height='7'/><rect fill='%23aaa' x='0' y='7' width='7' height='7'/><rect fill='%23fff' x='7' y='7' width='7' height='7'/></svg>"
            }*/

            MDKVideo {
                id: vid;
                visible: opacity > 0;
                opacity: loaded? 1 : 0;
                Ease on opacity { }
                anchors.fill: parent;
                property bool loaded: false;

                property bool stabEnabled: true;
                transform: [
                    Scale {
                        origin.x: vid.width / 2; origin.y: vid.height / 2;
                        xScale: vid.stabEnabled? 1 : vid.videoWidth  / Math.max(1, root.outWidth);
                        yScale: vid.stabEnabled? 1 : vid.videoHeight / Math.max(1, root.outHeight);
                    },
                    Rotation {
                        origin.x: vid.width / 2; origin.y: vid.height / 2;
                        angle: vid.stabEnabled? 0 : -vidInfo.videoRotation;
                    }
                ]

                function fovChanged() {
                    const fov = controller.current_fov;
                    // const ratio = controller.get_scaling_ratio(); // this shouldn't be called every frame because it locks the params mutex
                    currentFovText.text = qsTr("Zoom: %1").arg(fov > 0? (100 / fov).toFixed(2) + "%" : "---");
                    if (window.stab.fovSlider.field.value > 1) {
                        safeAreaRect.width = safeAreaRect.parent.width / window.stab.fovSlider.field.value;
                        safeAreaRect.height = safeAreaRect.parent.height / window.stab.fovSlider.field.value;
                    }
                }

                onCurrentFrameChanged: {
                    fovChanged();
                    controller.update_keyframe_values(timestamp);
                    window.motionData.orientationIndicator.updateOrientation(timeline.position * timeline.durationMs * 1000);
                }
                onMetadataLoaded: (md) => {
                    loaded = duration > 0;
                    videoLoader.active = false;
                    vidInfo.loader = false;
                    timeline.resetTrim();

                    controller.video_file_loaded(vid.url, vid);
                    window.motionData.filename = "";

                    if (root.pendingGyroflowData) {
                        Qt.callLater(root.loadGyroflowData, root.pendingGyroflowData);
                    } else {
                        controller.load_telemetry(vid.url, true, vid, timeline.getChart(), timeline.getKeyframesView());
                    }
                    vidInfo.loadFromVideoMetadata(md);
                    window.sync.customSyncTimestamps = [];
                    // for (var i in md) console.info(i, md[i]);
                }
                property bool errorShown: false;
                onMetadataChanged: {
                    if (vid.duration > 0) {
                        // Trigger seek to buffer the video frames
                        bufferTrigger.start();
                    } else if (!errorShown) {
                        messageBox(Modal.Error, qsTr("Failed to load the selected file, it may be unsupported or invalid."), [ { "text": qsTr("Ok") } ]);
                        errorShown = true;
                        dropText.loadingFile = "";
                        root.pendingGyroflowData = null;
                    }
                }
                Timer {
                    id: bufferTrigger;
                    interval: 500;
                    onTriggered: {
                        Qt.callLater(() => {
                            vid.currentFrame++;
                            Qt.callLater(() => vid.currentFrame--);
                        })
                    }
                }

                backgroundColor: "#111111";
                Component.onCompleted: {
                    controller.init_player(this);
                    Qt.callLater(() => {
                        if (!isCalibrator && openFileOnStart) {
                            root.loadFile(controller.path_to_url(openFileOnStart));
                        }
                    });
                }
                Rectangle {
                    border.color: styleVideoBorderColor;
                    border.width: 1 * dpiScale;
                    color: "transparent";
                    radius: 5 * dpiScale;
                    anchors.fill: parent;
                    anchors.margins: -border.width;
                }
                Item {
                    anchors.fill: parent;
                    layer.enabled: true;
                    opacity: root.safeArea && window.stab.fovSlider.field.value > 1 && stabEnabledBtn.checked? 1 : 0;
                    Ease on opacity { }
                    visible: opacity > 0;
                    Item {
                        id: safeAreaRect;
                        width: parent.width;
                        height: parent.height;
                        anchors.centerIn: parent;
                    }
                    Rectangle { x: -1; width: parent.width + 2; height: safeAreaRect.y; color: "#80000000"; } // Top
                    Rectangle { x: -1; y: safeAreaRect.y; width: safeAreaRect.x + 1; height: safeAreaRect.height; color: "#80000000"; } // Left
                    Rectangle { x: -1; y: safeAreaRect.y + safeAreaRect.height; width: parent.width + 2; height: parent.height - y; color: "#80000000"; } // Bottom
                    Rectangle { x: safeAreaRect.x + safeAreaRect.width; y: safeAreaRect.y; width: safeAreaRect.x + 1; height: safeAreaRect.height; color: "#80000000"; } // Right
                }
            }

            InfoMessage {
                width: vid.width;
                type: InfoMessage.Warning;
                visible: vid.loaded && !controller.lens_loaded && !isCalibrator;
                text: qsTr("Lens profile is not loaded, the results will not look correct. Please load a lens profile for your camera.");
            }
            MouseArea {
                anchors.fill: parent;
                onClicked: timeline.focus = true;
                onDoubleClicked: root.fullScreen = !root.fullScreen;
            }
        }
        Rectangle {
            id: dropRect;
            border.width: vid.loaded? 0 : (3 * dpiScale);
            border.color: style === "light"? Qt.darker(styleBackground, 1.3) : Qt.lighter(styleBackground, 2);
            anchors.fill: parent;
            anchors.margins: vid.loaded? 0 : (20 * dpiScale);
            anchors.topMargin: vid.loaded? 0 : (50 * dpiScale);
            anchors.bottomMargin: vid.loaded? 0 : (50 * dpiScale);
            color: styleBackground;
            radius: 5 * dpiScale;
            opacity: da.containsDrag? (vid.loaded? 0.8 : 0.3) : vid.loaded? 0 : 1.0;
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
                scale: dropText.contentWidth > (parent.width - 50 * dpiScale)? (parent.width - 50 * dpiScale) / dropText.contentWidth : 1.0;
            }
            DropTargetRect {
                visible: !dropText.loadingFile && !vid.loaded;
                anchors.fill: dropText;
                anchors.margins: -30 * dpiScale;
                scale: dropText.scale;
            }
            DropTargetRect {
                visible: !dropText.loadingFile && vid.loaded;
                anchors.margins: 5 * dpiScale;
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
            enabled: !queue.shown;

            onEntered: (drag) => {
                const ext = drag.urls[0].toString().split(".").pop().toLowerCase();
                drag.accepted = fileDialog.extensions.indexOf(ext) > -1;
            }
            onDropped: (drop) => {
                if (isCalibrator) {
                    calibrator_window.loadFile(drop.urls[0]);
                } else {
                    root.loadMultipleFiles(drop.urls, false);
                }
            }
        }
        LoaderOverlay {
            id: videoLoader;
            background: styleBackground;
            onActiveChanged: { vid.forceRedraw(); vid.fovChanged(); }
            canHide: render_queue.main_job_id > 0;
            onCancel: {
                if (render_queue.main_job_id > 0) {
                    render_queue.cancel_job(render_queue.main_job_id);
                } else {
                    controller.cancel_current_operation();
                }
            }
            onHide: {
                render_queue.main_job_id = 0;
                videoLoader.active = false;
            }
        }
        RenderQueue {
            id: queue;
            anchors.fill: vid.loaded? vidParent : dropRect;
            anchors.margins: 10 * dpiScale;
            onShownChanged: statistics.shown &= !shown;
        }
        Statistics {
            id: statistics;
            anchors.fill: vid.loaded? vidParent : dropRect;
            anchors.margins: 10 * dpiScale;
            onShownChanged: queue.shown &= !shown;
        }

        Connections {
            target: controller;
            function onCompute_progress(id: real, progress: real) {
                videoLoader.active = progress < 1;
                videoLoader.cancelable = false;
            }
            function onSync_progress(progress: real, ready: int, total: int) {
                videoLoader.active = progress < 1;
                videoLoader.currentFrame = ready;
                videoLoader.totalFrames = total;
                videoLoader.additional = "";
                videoLoader.text = videoLoader.active? qsTr("Analyzing %1...") : "";
                videoLoader.progress = videoLoader.active? progress : -1;
                videoLoader.cancelable = true;
            }
            function onLoading_gyro_progress(progress: real) {
                videoLoader.active = progress < 1;
                videoLoader.currentFrame = 0;
                videoLoader.totalFrames = 0;
                videoLoader.additional = "";
                videoLoader.text = videoLoader.active? qsTr("Loading gyro data %1...") : "";
                videoLoader.progress = videoLoader.active? progress : -1;
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
            visible: !root.fullScreen;

            Column {
                enabled: vid.loaded;
                anchors.verticalCenter: parent.verticalCenter;
                anchors.left: parent.left;
                anchors.leftMargin: 10 * dpiScale;
                spacing: 3 * dpiScale;
                Row {
                    BasicText {
                        text: timeline.timeAtPosition((vid.currentFrame + 1) / Math.max(1, vid.frameCount));
                        leftPadding: 0;
                        font.pixelSize: 14 * dpiScale;
                        anchors.verticalCenter: parent.verticalCenter;
                    }
                    BasicText {
                        text: `(${vid.currentFrame+1}/${vid.frameCount})`;
                        leftPadding: 5 * dpiScale;
                        font.pixelSize: 11 * dpiScale;
                        anchors.verticalCenter: parent.verticalCenter;
                    }
                }
                BasicText {
                    id: currentFovText;
                    font.pixelSize: 11 * dpiScale;
                    leftPadding: 0;
                }
            }

            Row {
                anchors.centerIn: parent;
                spacing: 5 * dpiScale;
                enabled: vid.loaded;
                Button { text: "["; font.bold: true; onClicked: timeline.setTrim(timeline.position, timeline.trimEnd); tooltip: qsTr("Trim start"); }
                Button { iconName: "chevron-left"; tooltip: qsTr("Previous frame"); onClicked: vid.currentFrame -= 1; }
                Button {
                    onClicked: if (vid.playing) vid.pause(); else vid.play();
                    tooltip: vid.playing? qsTr("Pause") : qsTr("Play");
                    iconName: vid.playing? "pause" : "play";
                }
                Button { iconName: "chevron-right"; tooltip: qsTr("Next frame"); onClicked: vid.currentFrame += 1; }
                Button { text: "]"; font.bold: true; onClicked: timeline.setTrim(timeline.trimStart, timeline.position); tooltip: qsTr("Trim end"); }
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
                    id: stabEnabledBtn;
                    iconName: "gyroflow";
                    onCheckedChanged: { vid.stabEnabled = checked; controller.stab_enabled = checked; vid.forceRedraw(); vid.fovChanged(); }
                    tooltip: qsTr("Toggle stabilization");
                }

                SmallLinkButton {
                    iconName: checked? "sound" : "sound-mute";
                    onClicked: vid.muted = !vid.muted;
                    tooltip: checked? qsTr("Mute") : qsTr("Unmute");
                    checked: !vid.muted;
                }

                ComboBox {
                    id: playbackRateCb;
                    model: ["0.13x", "0.25x", "0.5x", "1x", "2x", "4x", "5x", "8x", "10x", "20x"];
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

        Item { width: 1; height: 10 * dpiScale; visible: !root.fullScreen; }

        ResizablePanel {
            id: bottomPanel;
            direction: ResizablePanel.HandleUp;
            width: parent.width;
            color: "transparent";
            hr.height: 30 * dpiScale;
            hr.opacity: root.fullScreen? 0.1 : 1.0;
            additionalHeight: timeline.additionalHeight;
            defaultHeight: 165 * dpiScale;
            minHeight: (root.fullScreen? 50 : 100) * dpiScale;
            lastHeight: window.settings.value("bottomPanelSize" + (root.fullScreen? "-full" : ""), defaultHeight);
            onHeightAdjusted: window.settings.setValue("bottomPanelSize" + (root.fullScreen? "-full" : ""), height);
            Connections {
                target: root;
                function onFullScreenChanged() { bottomPanel.lastHeight = window.settings.value("bottomPanelSize" + (root.fullScreen? "-full" : ""), bottomPanel.defaultHeight); }
            }
            maxHeight: root.height - 50 * dpiScale;
            Timeline {
                id: timeline;
                durationMs: vid.duration;
                anchors.fill: parent;
                fullScreen: root.fullScreen;

                onTrimStartChanged: {
                    controller.set_trim_start(trimStart);
                    vid.setPlaybackRange(trimStart * vid.duration, trimEnd * vid.duration);
                }
                onTrimEndChanged: {
                    controller.set_trim_end(trimEnd);
                    vid.setPlaybackRange(trimStart * vid.duration, trimEnd * vid.duration);
                }
            }
        }
    }
}
