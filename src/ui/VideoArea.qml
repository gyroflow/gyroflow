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
    property alias fovOverviewBtn: fovOverviewBtn;
    property alias queue: queue.item;
    property alias statistics: statistics;
    property alias infoMessages: infoMessages;
    property alias gridGuide: gridGuide;

    property int outWidth: window? window.exportSettings.outWidth : 0;
    property int outHeight: window? window.exportSettings.outHeight : 0;

    property alias dropRect: dropRect;
    property bool isCalibrator: false;

    property var pendingGyroflowData: null;
    property url loadedFileUrl;

    property int fullScreen: 0;
    property string detectedCamera: "";
    property real additionalTopMargin: 0;

    property Menu.VideoInformation vidInfo: null;

    function loadGyroflowData(obj) {
        root.pendingGyroflowData = null;

        if (controller.loading_gyro_in_progress) {
            root.pendingGyroflowData = obj;
            controller.cancel_current_operation();
            // we'll get called again from telemetry_loaded
            return;
        }

        let urls = null;

        if (obj.toString() != '[object Object]') { // obj is url
            urls = controller.get_urls_from_gyroflow_file(obj);
        } else if (obj.project_file) {
            urls = controller.get_urls_from_gyroflow_file(obj.project_file);
        } else {
            urls = [
                obj.videofile,
                obj.gyro_source?.filepath || ""
            ];
        }

        const isCorrectVideoLoaded = urls[0] && vidInfo.filename == filesystem.get_filename(urls[0]);
        const isCorrectGyroLoaded  = urls[1] && window.motionData.filename == filesystem.get_filename(urls[1]);
        console.log("Video path:", urls[0], "(" + (isCorrectVideoLoaded? "loaded" : "not loaded") + ")", "Gyro path:", urls[1], "(" + (isCorrectGyroLoaded? "loaded" : "not loaded") + ")");

        if (urls[0] && !isCorrectVideoLoaded) {
            root.pendingGyroflowData = obj;
            console.log("Loading video file", urls[0]);
            loadFile(urls[0], false);
            if (controller.image_sequence_fps > 0) {
                vid.setFrameRate(controller.image_sequence_fps);
            }
            return;
        }
        if (urls[1] && !isCorrectGyroLoaded && filesystem.exists(urls[1])) {
            root.pendingGyroflowData = obj;
            console.log("Loading gyro file", urls[1]);
            window.motionData.lastSelectedFile = urls[1];
            controller.load_telemetry(urls[1], urls[0] == urls[1], window.videoArea.vid, -1);
            return;
        }

        controller.set_prevent_recompute(true);
        if (obj.toString() != '[object Object]') {
            // obj is url
            controller.import_gyroflow_file(obj);
        } else if (obj.project_file) {
            controller.import_gyroflow_file(obj.project_file);
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
                    if (info.hasOwnProperty("rotation")) {
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
            controller.set_prevent_recompute(false);
            Qt.callLater(controller.recompute_gyro);
            Qt.callLater(controller.recompute_threaded);
            Qt.callLater(timeline.updateDurations);
        }
        function onExternal_sdk_progress(percent: real, sdk_name: string, error_string: string, url: string) {
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
                        if (url == "ffmpeg_gpl") {
                            messageBox(Modal.Success, qsTr("Component was installed successfully.\nYou need to restart Gyroflow for changes to take effect.\nYour render queue and current file is saved automatically."), [ { text: qsTr("Ok") } ]);
                        } else {
                            loadFile(url, false);
                        }
                    } else {
                        if (Qt.platform.os == "osx") {
                            error_string += "\n" + qsTr("This is often caused by read-only file system.\nMake sure you copied the Gyroflow app to your Applications folder, instead of running from the .dmg directly.");
                        }
                        if (Qt.platform.os == "windows") {
                            error_string += "\n" + qsTr("This is often caused by read-only file system.\nIf you have Gyroflow in C:\\Program Files\\, then you'll need to run Gyroflow as Administrator in order to extract the SDK to the Gyroflow folder.");
                        }
                        messageBox(Modal.Error, error_string, [ { text: qsTr("Ok") } ]);
                    }
                }
            }
        }

        function onMp4_merge_progress(percent: real, error_string: string, url: url) {
            if (externalSdkModal !== null && externalSdkModal.loader !== null) {
                externalSdkModal.loader.visible = percent < 1;
                externalSdkModal.loader.active = percent < 1;
                externalSdkModal.loader.progress = percent;
                externalSdkModal.loader.text = qsTr("Merging files to %1 (%2)").arg("<b>" + filesystem.display_url(url) + "</b>");
                if (percent >= 1) {
                    externalSdkModal.close();
                    externalSdkModal = null;
                    window.isDialogOpened = false;
                    if (!error_string) {
                        loadFile(url, true);
                    } else {
                        messageBox(Modal.Error, error_string, [ { text: qsTr("Ok") } ]);
                    }
                }
            }
        }
        function onTelemetry_loaded(is_main_video: bool, filename: string, camera: string, additional_data: var) {
            console.log("Telemetry additional data:", JSON.stringify(additional_data));
            if (is_main_video) {
                root.detectedCamera = camera;
                vidInfo.updateEntry("Detected camera", camera || "---");

                let lens = "";
                if (additional_data.camera_identifier) {
                    const camera_id = additional_data.camera_identifier;
                    if (camera_id) {
                        if (camera_id.lens_model) { lens += camera_id.lens_model; }
                        if (camera_id.lens_info)  { lens += (lens? " " : "") + camera_id.lens_info; }
                    }
                }
                vidInfo.updateEntry("Detected lens", lens || "---");
                vidInfo.updateEntry("Contains gyro", additional_data.contains_motion? "Yes" : "No");
                // If source was detected, but gyro data is empty
                if (camera) {
                    if (!additional_data.contains_motion && !additional_data.contains_quats) {
                        messageBox(Modal.Warning, qsTr("File format was detected, but no motion data was found.\nThe camera probably doesn't record motion data in this particular shooting mode."), [ { "text": qsTr("Ok") } ]);
                    }
                    if (additional_data.contains_raw_gyro && !additional_data.contains_quats) timeline.setDisplayMode(0); // Switch to gyro view
                    if (!additional_data.contains_raw_gyro && additional_data.contains_quats) timeline.setDisplayMode(3); // Switch to quaternions view
                }

                if (additional_data.hasOwnProperty("cam_posture") && Math.abs(+additional_data.cam_posture.replace("CameraRotate", "")) > 0) {
                    vidInfo.updateEntryWithTrigger("Rotation", +additional_data.cam_posture.replace("CameraRotate", ""));
                }
                if (additional_data.hasOwnProperty("realtime_fps") && +additional_data.realtime_fps > 0) {
                    vidInfo.updateEntryWithTrigger("Frame rate", +additional_data.realtime_fps);
                }
            }
            if (+additional_data.sample_rate > 0.0 && Math.round(+additional_data.sample_rate) < 50) {
                messageBox(Modal.Warning, qsTr("Motion data sampling rate is too low (%1 Hz).\n50 Hz is an absolute minimum and we recommend at least 200 Hz.").arg(additional_data.sample_rate.toFixed(0)), [ { "text": qsTr("Ok") } ]);
            }
            if (root.pendingGyroflowData) {
                Qt.callLater(loadGyroflowData, root.pendingGyroflowData);
            } else {
                Qt.callLater(controller.recompute_threaded);
                if (is_main_video) {
                    controller.load_default_preset();
                }
            }
        }
        function onChart_data_changed() {
            timeline.triggerUpdateChart("");
        }
        function onZooming_data_changed() {
            timeline.triggerUpdateChart("8");
        }
        function updateKeyframesView() {
            controller.update_keyframes_view(timeline.getKeyframesView());
            controller.update_keyframe_values(vid.timestamp);
        }
        function onKeyframes_changed() {
            Qt.callLater(updateKeyframesView);
        }
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
    property Modal externalSdkModal: null;

    function loadFile(url: url, skip_detection: bool) {
        let filename = filesystem.get_filename(url);
        let folder = filesystem.get_folder(url);

        if (filename.endsWith(".gyroflow")) {
            return loadGyroflowData(url);
        }
        if (filename.endsWith(".RDC")) {
            // Assumes regular filesystem
            let parts = url.toString().split("/");
            parts.push(filename.replace(".RDC", "_001.R3D"));
            url = parts.join("/");
            filename = filesystem.get_filename(url);
            folder = filesystem.get_folder(url);
        }

        if (isMobile || filename.toLowerCase().endsWith(".r3d") || filename.toLowerCase().endsWith(".braw")) {
            // Preview resolution to 1080p
            if (isCalibrator && calibrator_window.lensCalib) {
                if (calibrator_window.lensCalib.previewResolution == 0) {
                    calibrator_window.lensCalib.previewResolution = 2;
                }
            } else {
                if (settings.value("previewResolution", -1) == -1 && window.advanced.previewResolution == 0) {
                    window.advanced.previewResolution = 2;
                }
            }
        }

        stabEnabledBtn.checked = false;

        if (controller.check_external_sdk(filename)) {
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

        if (!(/\.(png|jpg|exr|dng)$/i.test(filename) && filename.includes("%0"))) {
            root.loadedFileUrl = url;
        }
        if (!skip_detection) {
            let newUrl;
            if (newUrl = detectImageSequence(folder, filename)) {
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
            if (sequenceList = detectVideoSequence(folder, filename)) {
                const list = "<b>" + sequenceList.join(", ") + "</b>";
                const dlg = messageBox(Modal.Info, qsTr("Split recording has been detected, do you want to automatically join the files (%1) to create one full clip?").arg(list), [
                    { text: qsTr("Yes"), accent: true, clicked: function() {
                        dlg.btnsRow.children[0].enabled = false;
                        getOutputFile(folder, sequenceList[0], "_joined", "", true, function(outFolder, outFilename, outFullFileUrl) {
                            controller.mp4_merge(sequenceList.map(x => filesystem.get_file_url(folder, x, false).toString()), outFolder, outFilename);
                        });
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
        vidInfo.hasAccessToInputDirectory = folder.toString().length > 3;

        window.stab.fovSlider.value = 1.0;
        vid.loaded = false;
        videoLoader.active = true;
        vidInfo.loader = true;
        //vid.url = url;
        vid.errorShown = false;
        render_queue.editing_job_id = 0;
        controller.load_video(url, vid);
        if (!isCalibrator) {
            const suffix = window.advanced.defaultSuffix.text;
            window.outputFile.setFilename(filesystem.filename_with_suffix(filename, suffix).replace(/%0[0-9]+d/, ""));

            const preservedPath = settings.value("preservedOutputPath");
            if (window.exportSettings.preserveOutputPath.checked && preservedPath) {
                window.outputFile.setFolder(preservedPath);
            } else {
                window.outputFile.setFolder(folder);
            }
            window.exportSettings.updateCodecParams();
        }
        if (!root.pendingGyroflowData) {
            const gfFilename = filesystem.filename_with_extension(filename, "gyroflow");
            if (filesystem.exists_in_folder(folder, gfFilename)) {
                messageBox(Modal.Question, qsTr("There's a %1 file associated with this video, do you want to load it?").arg("<b>" + gfFilename + "</b>"), [
                    { text: qsTr("Yes"), clicked: function() {
                        Qt.callLater(() => loadFile(filesystem.get_file_url(folder, gfFilename, false), true));
                    } },
                    { text: qsTr("No"), accent: true },
                ]);
            }
        }

        dropText.loadingFile = filename;
        vidInfo.updateEntry("File name", filename);
        vidInfo.updateEntry("Detected camera", "---");
        vidInfo.updateEntry("Detected lens", "---");
        vidInfo.updateEntry("Contains gyro", "---");
        timeline.editingSyncPoint = false;
    }
    function loadMultipleFiles(urls: list<url>, skip_detection: bool) {
        if (urls.length == 1) {
            root.loadFile(urls[0], skip_detection);
        } else if (urls.length > 1) {
            const urlsCopy = [...urls];
            const dlg = messageBox(Modal.Question, qsTr("You have opened multiple files. What do you want to do?"), [
                { text: qsTr("Add to render queue"), clicked: () => {
                    queue.item.dt.loadFiles(urlsCopy);
                    queue.item.shown = true;
                } },
                { text: qsTr("Merge them into one video"), clicked: () => {
                    dlg.btnsRow.children[0].enabled = false;
                    dlg.btnsRow.children[1].enabled = false;
                    dlg.btnsRow.children[2].enabled = false;
                    const filename = filesystem.get_filename(urlsCopy[0]);
                    const folder = filesystem.get_folder(urlsCopy[0]);
                    getOutputFile(folder, filename, "_joined", "", true, function(outFolder, outFilename, outFullFileUrl) {
                        controller.mp4_merge(urlsCopy.map(x => x.toString()), outFolder, outFilename);
                    });
                    return false;
                } },
                { text: qsTr("Open the first file"), clicked: () => {
                    root.loadFile(urlsCopy[0], skip_detection);
                } },
                { text: qsTr("Cancel") },
            ]);
            externalSdkModal = dlg;
            dlg.addLoader();
        }
    }

    function askForOutputLocation(folder: url, filename: string, choice: bool, cb) {
        const dlg = messageBox(Modal.Question, qsTr("Please enter the output path:"), [
            { text: qsTr("Ok"), accent: true, clicked: function() {
                if (choice) {
                    if (dlg.mainColumn.children[1].children[0].checked) { cb("", ""); }
                    if (dlg.mainColumn.children[1].children[1].checked) { const opf = dlg.mainColumn.children[1].children[3]; cb(opf.folderUrl, opf.filename, opf.fullFileUrl); }
                } else {
                    const opf = dlg.mainColumn.children[1];
                    cb(opf.folderUrl, opf.filename, opf.fullFileUrl);
                }
            } },
            { text: qsTr("Cancel") },
        ]);

        if (choice) {
            let col = Qt.createQmlObject(`import QtQuick; import "components/";
                Column {
                    width: parent.width;
                    RadioButton { checked: true; }
                    RadioButton { id: custom; }
                    Item { height: 10 * dpiScale; width: 1; }
                    OutputPathField { enabled: custom.checked; folderOnly: true; }
                }`, dlg.mainColumn, "dlgRadios");
            col.children[0].text = qsTr("Same as the original file");
            col.children[1].text = qsTr("Custom path");
            col.children[3].setFolder(folder);
        } else {
            const opf = Qt.createComponent("components/OutputPathField.qml").createObject(dlg.mainColumn, { });
            opf.setFolder(folder);
            opf.setFilename(filename);
        }
    }
    function getOutputFile(folder: url, filename: string, suffix: string, extension: string, ask: bool, cb) {
        if (suffix) filename = filesystem.filename_with_suffix(filename, suffix);
        if (extension) filename = filesystem.filename_with_extension(filename, extension);
        if (ask) {
            askForOutputLocation(folder, filename, false, cb);
        } else {
            cb(folder, filename);
        }
    }

    function detectImageSequence(folder: url, filename: string) {
        if (!filename.includes("%0")) {
            controller.image_sequence_start = 0;
            controller.image_sequence_fps = 0;
        }
        if (/\d+\.(png|jpg|exr|dng)$/i.test(filename)) {
            let firstNum = filename.match(/(\d+)\.(png|jpg|exr|dng)$/i);
            if (firstNum[1]) {
                const ext = firstNum[2];
                firstNum = firstNum[1];
                const firstNumNum = parseInt(firstNum, 10);
                for (let i = firstNumNum + 1; i < firstNumNum + 5; ++i) { // At least 5 frames
                    const newNum = i.toString().padStart(firstNum.length, '0');
                    const newName = filename.replace(firstNum + "." + ext, newNum + "." + ext);
                    if (!filesystem.exists_in_folder(folder, newName)) {
                        return false;
                    }
                }
                controller.image_sequence_start = firstNumNum;
                return filesystem.get_file_url(folder, filename.replace(`${firstNum}.${ext}`, `%0${firstNum.length}d.${ext}`), false);
            }
        }
        return false;
    }
    function detectVideoSequence(folder: url, filename: string) {
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
        ];
        for (const x of patterns) {
            let match = filename.match(x[0]);
            if (match && match[1]) {
                let list = [];
                const firstNum = parseInt(match[2], 10);
                for (let i = firstNum; i < firstNum + 99; ++i) { // Max 99 parts
                    const newName = filename.replace(match[1], x[1](match[1], i));
                    if (filesystem.exists_in_folder(folder, newName)) {
                        list.push(newName);
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

    Item {
        id: vidParentParent;
        width: parent.width;
        height: parent.height - (root.fullScreen || window.isMobileLayout? 0 : tlcol.height);
        Item {
            id: vidParent;
            property real orgW: root.outWidth || vid.videoWidth;
            property real orgH: root.outHeight || vid.videoHeight;
            property real ratio: orgW / Math.max(1, orgH);
            property real w: parent.width  - (root.fullScreen? 0 : 20 * dpiScale);
            property real h: parent.height - (root.fullScreen? 0 : 20 * dpiScale);

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

                property bool stabEnabled: stabEnabledBtn.checked;
                transform: [
                    Scale {
                        origin.x: vid.width / 2; origin.y: vid.height / 2;
                        xScale: vid.stabEnabled? 1 : Math.max(1.0, (root.outHeight / Math.max(1, root.outWidth)) / ((vid.videoHeight * window.lensProfile.input_vertical_stretch) / Math.max(1, vid.videoWidth * window.lensProfile.input_horizontal_stretch))) * (fovOverviewBtn.checked? 0.5 : 1);
                        yScale: vid.stabEnabled? 1 : Math.max(1.0, (root.outWidth / Math.max(1, root.outHeight)) / ((vid.videoWidth * window.lensProfile.input_horizontal_stretch) / Math.max(1, vid.videoHeight * window.lensProfile.input_vertical_stretch))) * (fovOverviewBtn.checked? 0.5 : 1);
                    },
                    Rotation {
                        origin.x: vid.width / 2; origin.y: vid.height / 2;
                        angle: vid.stabEnabled? 0 : -vidInfo.videoRotation;
                    }
                ]

                function fovChanged() {
                    const fov = controller.current_fov;
                    const focal_length = controller.current_focal_length;
                    const crop_factor = window.lensProfile?.cropFactor || 1.0;
                    // const ratio = controller.get_scaling_ratio(); // this shouldn't be called every frame because it locks the params mutex
                    currentFovText.text = qsTr("Zoom: %1").arg(fov > 0? (100 / fov).toFixed(2) + "%" : "---");

                    if (+focal_length > 0) {
                        const fl = +focal_length / fov;
                        currentFovText.text += "\n" + qsTr("Focal length: %1 mm").arg(fl.toFixed(2));
                        if (crop_factor && crop_factor != 1.0) {
                            currentFovText.text += " (" + qsTr("full frame equiv.: %1 mm").arg((fl * crop_factor).toFixed(2)) + ")";
                        }
                    }
                }

                onCurrentFrameChanged: {
                    fovChanged();
                    controller.update_keyframe_values(timestamp);
                    window.motionData.orientationIndicator.updateOrientation(timeline.position * timeline.durationMs * 1000);
                }
                onMetadataLoaded: (md) => {
                    Qt.callLater(fileLoaded, md);
                }
                function fileLoaded(md: var) {
                    loaded = duration > 0;
                    videoLoader.active = false;
                    vidInfo.loader = false;
                    timeline.resetTrim();
                    timeline.resetZoom();

                    controller.video_file_loaded(vid);
                    window.motionData.filename = "";

                    if (root.pendingGyroflowData) {
                        Qt.callLater(root.loadGyroflowData, root.pendingGyroflowData);
                    } else {
                        controller.load_telemetry(root.loadedFileUrl, true, vid, -1);
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
                    interval: 150;
                    onTriggered: {
                        if (!vid.loaded) bufferTrigger.start();
                        Qt.callLater(() => {
                            vid.currentFrame++;
                            Qt.callLater(() => vid.currentFrame = 0);
                            if (vid.loaded) {
                                stabEnabledBtn.checked = true;
                                vid.volume = volumeSlider.value / 100.0;
                            }
                        });
                    }
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
            }

            TapHandler {
                onTapped: timeline.focus = true;
                onDoubleTapped: root.fullScreen = root.fullScreen? 0 : 1;
            }
            GridGuide {
                id: gridGuide;
                anchors.fill: vid;
                canShow: vid.loaded;
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
                text: loadingFile? qsTr("Loading %1...").arg(loadingFile) : (Qt.platform.os == "ios" || Qt.platform.os == "android"? qsTr("Click here to open a video file") : qsTr("Drop video file here"));
                font.pixelSize: (window.isMobileLayout? 23 : 30) * dpiScale;
                anchors.centerIn: parent;
                leftPadding: 0;
                scale: dropText.contentWidth > (parent.width - 50 * dpiScale)? (parent.width - 50 * dpiScale) / dropText.contentWidth : 1.0;
            }
            ItemLoader {
                anchors.fill: dropText;
                anchors.margins: -30 * dpiScale;
                visible: !dropText.loadingFile && !vid.loaded;
                scale: dropText.scale;
                sourceComponent: Component { DropTargetRect { } }
            }
            ItemLoader {
                anchors.fill: parent;
                anchors.margins: 5 * dpiScale;
                visible: !dropText.loadingFile && vid.loaded;
                sourceComponent: Component { DropTargetRect { } }
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
            enabled: queue.item && !queue.item.shown;

            onEntered: (drag) => {
                const ext = drag.urls[0].toString().split(".").pop().toLowerCase();
                drag.accepted = fileDialog.extensions.indexOf(ext) > -1 || ext == "rdc";
            }
            onDropped: (drop) => {
                if (isCalibrator) {
                    calibrator_window.loadFiles(drop.urls);
                } else {
                    root.loadMultipleFiles(drop.urls, false);
                }
            }
        }
    }

    Column {
        id: tlcol;
        width: parent.width;
        anchors.horizontalCenter: parent.horizontalCenter;
        anchors.bottom: parent.bottom;
        anchors.bottomMargin: areButtonsUp? 0 : 5 * dpiScale;
        spacing: root.fullScreen || window.isMobileLayout? 0 : 10 * dpiScale;
        property bool areButtonsUp: !window.isMobileLayout;
        onAreButtonsUpChanged: {
            buttonsArea.parent = null;
            bottomPanel.parent = null;
            if (areButtonsUp) {
                buttonsArea.parent = tlcol;
                bottomPanel.parent = tlcol;
            } else {
                bottomPanel.parent = tlcol;
                buttonsArea.parent = tlcol;
            }
        }
        Component.onCompleted: areButtonsUpChanged();

        Item {
            id: buttonsArea;
            width: parent? parent.width : 0;
            height: 40 * dpiScale;
            visible: !root.fullScreen;

            Rectangle {
                visible: window.isMobileLayout || !middleButtons.willFit;
                color: styleBackground;
                opacity: 0.8;
                radius: 5 * dpiScale;
                anchors.fill: textCol;
                anchors.margins: -4 * dpiScale;
            }
            Column {
                id: textCol;
                enabled: vid.loaded;
                y: middleButtons.willFit? ((parent.height - height) / 2) : -buttonsArea.y - tlcol.y + 7 * dpiScale + ((main_window.safeAreaMargins.top || 0) * 0.8);
                anchors.left: parent.left;
                anchors.leftMargin: 10 * dpiScale;
                spacing: 3 * dpiScale;
                property real widthPadded: Math.ceil(width / (20 * dpiScale)) * (20 * dpiScale);
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

            Item {
                id: middleButtons;
                property real availableWidth: parent.width - textCol.widthPadded - rightButtons.width - 40 * dpiScale;
                width: parent.width - (willFit? textCol.widthPadded + rightButtons.width + 40 * dpiScale : 0);
                height: parent.height;
                x: willFit? textCol.x + textCol.widthPadded + 10 * dpiScale : 0;
                property bool willFit: availableWidth > children[0].width;
                Row {
                    anchors.centerIn: parent;
                    spacing: 5 * dpiScale;
                    enabled: vid.loaded;
                    Button { text: "["; font.bold: true; onClicked: timeline.setTrim(timeline.position, timeline.trimEnd); tooltip: qsTr("Trim start"); transparentOnMobile: true; }
                    Button { iconName: "chevron-left"; tooltip: qsTr("Previous frame"); onClicked: vid.seekToFrameDelta(-1); transparentOnMobile: true; }
                    Button {
                        onClicked: { if (vid.playing) vid.pause(); else vid.play(); }
                        tooltip: vid.playing? qsTr("Pause") : qsTr("Play");
                        iconName: vid.playing? "pause" : "play";
                        transparentOnMobile: true;
                    }
                    Button { iconName: "chevron-right"; tooltip: qsTr("Next frame"); onClicked: vid.seekToFrameDelta(1); transparentOnMobile: true; }
                    Button { text: "]"; font.bold: true; onClicked: timeline.setTrim(timeline.trimStart, timeline.position); tooltip: qsTr("Trim end"); transparentOnMobile: true; }
                    Button { visible: isMobile; iconName: "menu"; onClicked: timeline.toggleContextMenu(this); tooltip: qsTr("Show timeline menu"); transparentOnMobile: true; leftPadding: 10 * dpiScale; rightPadding: 10 * dpiScale; }
                }
            }
            Rectangle {
                visible: window.isMobileLayout || !middleButtons.willFit;
                color: styleBackground;
                opacity: 0.8;
                radius: 5 * dpiScale;
                anchors.fill: rightButtons;
                anchors.margins: -4 * dpiScale;
            }
            Row {
                id: rightButtons;
                enabled: vid.loaded;
                spacing: 5 * dpiScale;
                y: middleButtons.willFit? ((parent.height - height) / 2) : -buttonsArea.y - tlcol.y + ((main_window.safeAreaMargins.top || 0) * 0.8);
                onYChanged: root.additionalTopMargin = middleButtons.willFit? 0 : Math.max(height, textCol.height) + 2*4 * dpiScale + ((main_window.safeAreaMargins.top || 0) * 0.8);
                anchors.right: parent.right;
                anchors.rightMargin: 10 * dpiScale;
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
                    id: fovOverviewBtn;
                    iconName: "fov-overview";
                    checked: false;
                    onCheckedChanged: { controller.fov_overview = checked; vid.forceRedraw(); }
                    tooltip: qsTr("Toggle stabilization overview");
                }

                SmallLinkButton {
                    id: stabEnabledBtn;
                    iconName: "gyroflow";
                    onCheckedChanged: { controller.stab_enabled = checked; vid.forceRedraw(); vid.fovChanged(); }
                    tooltip: qsTr("Toggle stabilization");
                }

                SmallLinkButton {
                    id: muteBtn;
                    iconName: checked? "sound" : "sound-mute";
                    tooltip: checked? qsTr("Mute") : qsTr("Unmute");
                    checked: !vid.muted;

                    ContextMenuMouseArea {
                        underlyingItem: muteBtn;
                        cursorShape: Qt.PointingHandCursor;
                        onContextMenu: (isHold, x, y) => { volumePopup.open(); if (isHold) vid.muted = !vid.muted; }
                    }
                    onClicked: () => { vid.muted = !vid.muted; }
                    Popup {
                        id: volumePopup;
                        width: volumeLabel.width + 25 * dpiScale;
                        height: 30 * dpiScale;
                        x: -width + muteBtn.width;
                        y: -height;
                        Label {
                            id: volumeLabel;
                            anchors.centerIn: parent;
                            text: qsTr("Volume");
                            position: Label.LeftPosition;
                            width: t.width + volumeSlider.width;
                            Slider {
                                id: volumeSlider;
                                width: 200 * dpiScale;
                                unit: "%";
                                from: 0;
                                to: 100;
                                value: window.settings.value("volume", 100);
                                precision: 0;
                                onValueChanged: { vid.volume = value / 100.0; window.settings.setValue("volume", value); }
                            }
                        }
                    }
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

        ResizablePanel {
            id: bottomPanel;
            direction: ResizablePanel.HandleUp;
            width: parent? parent.width : 0;
            color: "transparent";
            hr.height: 30 * dpiScale;
            hr.opacity: root.fullScreen || window.isMobileLayout? 0.1 : 1.0;
            additionalHeight: timeline.additionalHeight;
            defaultHeight: (window.isMobileLayout? 50 : 165) * dpiScale;
            minHeight: (root.fullScreen || window.isMobileLayout? 50 : 100) * dpiScale;
            lastHeight: window.settings.value("bottomPanelSize" + (root.fullScreen? "-full" : ""), defaultHeight);
            onHeightAdjusted: window.settings.setValue("bottomPanelSize" + (root.fullScreen? "-full" : ""), height);
            Connections {
                target: root;
                function onFullScreenChanged() {
                    bottomPanel.lastHeight = window.settings.value("bottomPanelSize" + (root.fullScreen? "-full" : ""), bottomPanel.defaultHeight);
                    if (root.fullScreen == 2) {
                        main_window.visibility = Window.FullScreen;
                    } else {
                        if (main_window.visibility == Window.FullScreen) main_window.visibility = Window.Windowed;
                    }
                }
            }
            visible: root.fullScreen != 2;
            maxHeight: root.height - 50 * dpiScale;
            Timeline {
                id: timeline;
                durationMs: vid.duration;
                scaledFps: vid.frameRate;
                anchors.fill: parent;
                fullScreen: root.fullScreen;
                visible: vid.loaded || !window.isMobileLayout;

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
    Item {
        width: vidParentParent.width;
        height: vidParentParent.height;
        LoaderOverlay {
            id: videoLoader;
            background: styleBackground;
            verticalOffset: window.isMobileLayout? -bottomPanel.height / 2 : 0;
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
        Column {
            id: infoMessages;
            width: parent.width;
            spacing: 5 * dpiScale;
            visible: children.length > 0;
            y: root.additionalTopMargin;
            InfoMessage {
                type: InfoMessage.Warning;
                visible: vid.loaded && !controller.lens_loaded && !isCalibrator;
                text: qsTr("Lens profile is not loaded, the results will not look correct. Please load a lens profile for your camera.");
            }
        }
    }
    Loader {
        id: queue;
        asynchronous: true;
        anchors.fill: vidParentParent;
        anchors.margins: 10 * dpiScale;
        sourceComponent: Component {
            RenderQueue {
                onShownChanged: if (statistics.item) statistics.item.shown &= !shown;
            }
        }
    }
    Loader {
        id: statistics;
        asynchronous: true;
        active: false;
        anchors.fill: vidParentParent;
        anchors.margins: 10 * dpiScale;
        onStatusChanged: if (status == Loader.Ready) statistics.item.shown = true;
        sourceComponent: Component {
            Statistics {
                onShownChanged: queue.item.shown &= !shown;
            }
        }
    }
}
