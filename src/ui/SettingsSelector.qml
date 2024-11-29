// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC

import "components/"
import "Util.js" as Util;

Modal {
    id: root;
    isWide: true;
    widthRatio: isMobile? (isLandscape? 0.8 : 0.95) : 0.6;
    iconType: Modal.NoIcon;

    property string type: "apply";

    signal apply(obj: var);

    property var desc: [
    { // Left column
        "Video|video_info": {
            "Rotation":   ["rotation"],
            "Frame rate": ["fps_scale", "vfr_fps"],
        },
        "Lens profile": ["calibration_data", "light_refraction_coefficient"],
        "Motion data|gyro_source": {
            "Low pass filter":    ["lpf"],
            "Median filter":      ["mf"],
            "Rotation":           ["rotation", "acc_rotation"],
            "Gyro bias":          ["gyro_bias"],
            "IMU orientation":    ["imu_orientation"],
            "Integration method": ["integration_method"],
        },
        "Trim range": ["trim_ranges_ms"],
        "Offsets":    ["offsets"],
        "Keyframes":  ["keyframes"]
    },
    { // Right column
        "Synchronization|synchronization": {
            "Rough gyro offset":          ["initial_offset", "initial_offset_inv"],
            "Sync search size":           ["search_size", "calc_initial_fast"],
            "Max sync points":            ["max_sync_points"],
            "Do autosync":                ["do_autosync"],
            "Advanced":                   ["every_nth_frame", "time_per_syncpoint", "of_method", "offset_method", "pose_method", "auto_sync_points"]
        },
        "Stabilization|stabilization": {
            "FOV":                        ["fov"],
            "Smoothing params":           ["method", "smoothing_params"],
            "Horizon lock":               ["horizon_lock_amount", "horizon_lock_roll", "use_gravity_vectors"],
            "Rolling shutter correction": ["frame_readout_time", "frame_readout_direction"],
            "Zooming":                    ["adaptive_zoom_window", "adaptive_zoom_center_offset", "adaptive_zoom_method", "additional_rotation", "additional_translation", "max_zoom", "max_zoom_iterations"],
            "Lens correction strength":   ["lens_correction_amount"],
            "Video speed":                ["video_speed", "video_speed_affects_smoothing", "video_speed_affects_zooming", "video_speed_affects_zooming_limit"],
        },
        "Export settings|output": {
            "Codec":       ["codec", "codec_options", "bitrate", "use_gpu"],
            "Audio":       ["audio"],
            "Output size": ["output_width", "output_height"],
            "Output path": ["output_folder", "output_filename"],
            "Advanced":    ["encoder_options", "metadata", "keyframe_distance", "preserve_other_tracks", "pad_with_black", "export_trims_separately", "audio_codec", "interpolation"],
        },
        "Advanced": {
            "Background":           ["background_color", "background_mode", "background_margin", "background_margin_feather"],
            "Playback speed":       ["playback_speed"],
            "Playback mute status": ["muted"]
        }
    }];

    property var defaultOff: ["trim_ranges_ms", "offsets", "video_infofps_scale", "video_inforotation", "synchronizationdo_autosync"];

    text: type == "preset"? qsTr("Select settings you want to include in the preset")
        : type == "apply"? qsTr("Select settings you want to apply to all items in the render queue")
        : qsTr("Select fields to include in the exported file");
    t.font.bold: true;
    t.font.pixelSize: 18 * dpiScale;

    function getData(group: int): list<var> {
        return Object.keys(root.desc[group]).map((v) => {
            const text = v.split("|")[0];
            const group_name = v.split("|")[1] || "";
            const properties = root.desc[group][v];
            const hasChildren = !Array.isArray(properties);
            let finalVal = hasChildren? Object.keys(properties) : [text];
            finalVal = finalVal.map((vv) => [vv, group_name, hasChildren? properties[vv] : properties]);
            return [text, finalVal];
        });
    }
    Component.onCompleted: {
        if (root.type == "apply") delete root.desc[1]["Synchronization|synchronization"];
        if (root.type == "gyro_csv") {
            groupsRepeater.model = [root.getData(0), root.getData(1), root.getData(2)];
        } else {
            groupsRepeater.model = [root.getData(0), root.getData(1)];
        }
        QT_TR_NOOP("Video");
            QT_TR_NOOP("Rotation");
            QT_TR_NOOP("Frame rate");
        QT_TR_NOOP("Lens profile");
        QT_TR_NOOP("Motion data");
            QT_TR_NOOP("Low pass filter");
            QT_TR_NOOP("Median filter");
            QT_TR_NOOP("Rotation");
            QT_TR_NOOP("Gyro bias");
            QT_TR_NOOP("IMU orientation");
            QT_TR_NOOP("Integration method");
        QT_TR_NOOP("Trim range");
        QT_TR_NOOP("Offsets");
        QT_TR_NOOP("Keyframes");
        QT_TR_NOOP("Synchronization");
            QT_TR_NOOP("Rough gyro offset");
            QT_TR_NOOP("Sync search size");
            QT_TR_NOOP("Max sync points");
            QT_TR_NOOP("Do autosync");
            QT_TR_NOOP("Advanced");
        QT_TR_NOOP("Stabilization");
            QT_TR_NOOP("FOV");
            QT_TR_NOOP("Smoothing params");
            QT_TR_NOOP("Horizon lock");
            QT_TR_NOOP("Rolling shutter correction");
            QT_TR_NOOP("Zooming");
            QT_TR_NOOP("Lens correction strength");
            QT_TR_NOOP("Video speed");
        QT_TR_NOOP("Export settings");
            QT_TR_NOOP("Codec");
            QT_TR_NOOP("Audio");
            QT_TR_NOOP("Output path");
            QT_TR_NOOP("Output size");
            QT_TR_NOOP("Advanced");
        QT_TR_NOOP("Advanced");
            QT_TR_NOOP("Background");
            QT_TR_NOOP("Playback speed");
            QT_TR_NOOP("Playback mute status");

        QT_TR_NOOP("Original");
        QT_TR_NOOP("Stabilized");
        QT_TR_NOOP("Zooming");
            QT_TR_NOOP("Gyroscope");
            QT_TR_NOOP("Accelerometer");
            QT_TR_NOOP("Quaternion");
            QT_TR_NOOP("Euler angles");
            QT_TR_NOOP("Minimal FOV scale");
            QT_TR_NOOP("Smoothed FOV scale");
            QT_TR_NOOP("Focal length (if available)");
    }

    Item { width: 1; height: 10 * dpiScale; }

    Row {
        id: sectionsArea;
        width: parent.width;
        spacing: 15 * dpiScale;
        function forAllCheckboxes(node: QtObject, cb: var): void {
            for (let i = node.children.length; i > 0; --i) {
                const child = node.children[i - 1];
                if (child) {
                    if (child instanceof CheckBox) {
                        cb(child);
                    }
                    forAllCheckboxes(child, cb);
                }
            }
        }
        Repeater {
            id: groupsRepeater;
            model: 2;
            Column {
                width: (parent.width - sectionsArea.spacing * (groupsRepeater.count - 1)) / groupsRepeater.count;
                spacing: 15 * dpiScale;
                Repeater {
                    model: modelData;
                    Rectangle {
                        radius: 8 * dpiScale;
                        color: Qt.lighter(styleBackground, 1.2);
                        width: parent.width;
                        height: groupCb.height + 20 * dpiScale;
                        Column {
                            id: groupCb;
                            x: 10 * dpiScale;
                            y: x;
                            spacing: 1 * dpiScale;
                            width: parent.width;
                            BasicText {
                                text: qsTr(modelData[0] || "");
                                font.bold: true;
                                font.pixelSize: 18 * dpiScale;
                                leftPadding: 0;
                                width: parent.width;
                                MouseArea {
                                    anchors.fill: parent;
                                    cursorShape: Qt.PointingHandCursor;
                                    onClicked: (mouse) => {
                                        const invert = mouse.modifiers & Qt.ControlModifier;
                                        sectionsArea.forAllCheckboxes(sectionsArea, function(cb) {
                                            if (invert  && cb.parent == groupCb) return;
                                            if (!invert && cb.parent != groupCb) return;
                                            if (invert && root.defaultOff.includes(cb.group + cb.props[0])) return;
                                            cb.checked = !cb.checked;
                                        });
                                    }
                                }
                            }
                            Item { width: 1; height: 4 * dpiScale; }
                            Repeater {
                                model: modelData[1];
                                CheckBox {
                                    text: qsTr(modelData[0]);
                                    checked: !root.defaultOff.includes(group + props[0]);
                                    property string group: modelData[1];
                                    property var props: modelData[2];
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    BasicText {
        visible: root.type == "preset";
        text: qsTr("Hint: You can have your presets in the lens profile search box, if you save your preset (`.gyroflow` file) in the directory with lens profiles.") + "\n\n" +
              qsTr("You can also save your preset as `default.gyroflow` in the directory with lens profiles and it will be always applied to every loaded video file (also in plugins).");
        color: styleTextColor;
        textFormat: Text.MarkdownText;
    }
    Column {
        visible: root.type == "preset";
        width: parent.width;
        RadioButton {
            id: saveToFile;
            checked: true;
            text: qsTr("Save to file")
        }
        RadioButton {
            id: saveToLensProfiles;
            text: qsTr("Save to lens profile directory");
        }
        RadioButton {
            id: saveAsDefaultPreset;
            text: qsTr("Save as default preset");
        }
    }

    Column {
        visible: root.type == "gyro_csv";
        width: parent.width;
        RadioButton {
            id: exportAllSamples;
            checked: true;
            text: qsTr("Export all samples");
        }
        RadioButton {
            id: exportPerFrame;
            text: qsTr("Export one sample per frame");
            onCheckedChanged: {
                sectionsArea.forAllCheckboxes(sectionsArea, function(cb) {
                    if (cb.props[0] == "gyroscope" || cb.props[0] == "accelerometer") {
                        cb.enabled = !checked;
                        if (!cb.enabled && cb.checked) cb.checked = false;
                    }
                });
            }
        }
    }
    BasicText {
        visible: root.type == "gyro_csv" && exportPerFrame.checked;
        text: qsTr("When exporting one sample per frame, it's the sample in the middle of the frame, and it ignores rolling shutter correction.");
        color: styleTextColor;
    }

    Item { width: 1; height: 10 * dpiScale; }

    onClicked: (index) => {
        if (index == 0) { // Save/Apply
            let finalObj = { };

            if (type == "preset") {
                finalObj.save_type = saveToFile.checked? "file" : saveToLensProfiles.checked? "lens" : "default";
            } else if (type == "gyro_csv") {
                finalObj.export_all_samples = exportAllSamples.checked;
            }

            sectionsArea.forAllCheckboxes(sectionsArea, function(cb) {
                for (const x of cb.props) {
                    if (cb.group) {
                        if (!finalObj[cb.group]) finalObj[cb.group] = { };
                        finalObj[cb.group][x] = cb.checked && cb.enabled;
                    } else {
                        finalObj[x] = cb.checked;
                    }
                }
            });

            root.apply(finalObj);
        }
        root.opened = false;
        root.destroy(1000);
    }
    buttons: [type == "apply"? qsTr("Apply") : qsTr("Save"), qsTr("Cancel")];
    accentButton: 0;

    function copyObj(from: var, by: var, to: var): void {
        for (const key in by) {
            if (typeof by[key] === "boolean") {
                if (by[key]) {
                    to[key] = from[key];
                }
            } else {
                to[key] = { };
                if (from.hasOwnProperty(key))
                    copyObj(from[key], by[key], to[key]);
            }
        }
    }
    function getFilteredObject(source: var, desc: var): var {
        let finalData = { version: 2 };
        copyObj(source, desc, finalData);
        // Cleanup empty objects
        for (const key in finalData) {
            if (typeof finalData[key] === "object" && !Array.isArray(finalData[key]) && Object.keys(finalData[key]).length == 0) {
                delete finalData[key];
            }
        }
        return finalData;
    }

    function loadSelection(obj: var): void {
        if (obj.hasOwnProperty("export_all_samples")) {
            if (obj.export_all_samples) {
                exportAllSamples.checked = true;
            } else {
                exportPerFrame.checked = true;
            }
        }
        if (obj.hasOwnProperty("save_type")) {
            if (obj.save_type == "file") saveToFile.checked = true;
            if (obj.save_type == "lens") saveToLensProfiles.checked = true;
            if (obj.save_type == "default") saveAsDefaultPreset.checked = true;
        }

        sectionsArea.forAllCheckboxes(sectionsArea, function(cb) {
            for (const x of cb.props) {
                if (cb.group) {
                    cb.checked = obj[cb.group] && obj[cb.group][x];
                } else {
                    cb.checked = obj[x];
                }
            }
        });
        exportPerFrame.checkedChanged();
    }
}
