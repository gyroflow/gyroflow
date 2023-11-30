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

    property bool isPreset: true;

    signal apply(obj: var);

    property var desc: [
    { // Left column
        "Video|video_info": {
            "Rotation":   ["rotation"],
            "Frame rate": ["fps_scale", "vfr_fps"],
        },
        "Lens profile": ["calibration_data"],
        "Motion data|gyro_source": {
            "Low pass filter":    ["lpf"],
            "Rotation":           ["rotation", "acc_rotation"],
            "Gyro bias":          ["gyro_bias"],
            "IMU orientation":    ["imu_orientation"],
            "Integration method": ["integration_method"],
        },
        "Trim range": ["trim_start", "trim_end"],
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
            "Rolling shutter correction": ["frame_readout_time"],
            "Zooming":                    ["adaptive_zoom_window", "adaptive_zoom_center_offset", "adaptive_zoom_method"],
            "Lens correction strength":   ["lens_correction_amount"],
            "Video speed":                ["video_speed", "video_speed_affects_smoothing", "video_speed_affects_zooming"],
        },
        "Export settings|output": {
            "Codec":       ["codec", "codec_options", "bitrate", "use_gpu"],
            "Audio":       ["audio"],
            "Output size": ["output_width", "output_height"],
            "Output path": ["output_folder", "output_filename"],
            "Advanced":    ["encoder_options", "metadata", "keyframe_distance", "preserve_other_tracks", "pad_with_black", "audio_codec"],
        },
        "Advanced": {
            "Background":           ["background_color", "background_mode", "background_margin", "background_margin_feather"],
            "Playback speed":       ["playback_speed"],
            "Playback mute status": ["muted"]
        }
    }];

    property var defaultOff: ["trim_start", "offsets", "video_infofps_scale", "video_inforotation", "synchronizationdo_autosync"];

    text: isPreset? qsTr("Select settings you want to include in the preset")
                  : qsTr("Select settings you want to apply to all items in the render queue");
    t.font.bold: true;
    t.font.pixelSize: 18 * dpiScale;

    function getData(group: int) {
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
        if (!root.isPreset) delete root.desc[1]["Synchronization|synchronization"];
        groupsRepeater.model = [root.getData(0), root.getData(1)];
        QT_TR_NOOP("Video");
            QT_TR_NOOP("Rotation");
            QT_TR_NOOP("Frame rate");
        QT_TR_NOOP("Lens profile");
        QT_TR_NOOP("Motion data");
            QT_TR_NOOP("Low pass filter");
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
    }

    Item { width: 1; height: 10 * dpiScale; }

    Row {
        id: sectionsArea;
        width: parent.width;
        spacing: 15 * dpiScale;
        function forAllCheckboxes(node, cb) {
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
                width: parent.width / 2 - sectionsArea.spacing / 2;
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
        visible: root.isPreset;
        text: qsTr("Hint: You can have your presets in the lens profile search box, if you save your preset (`.gyroflow` file) in the `camera_presets` directory.") + "\n\n" +
              qsTr("You can also save your preset as `default.gyroflow` in the `camera_presets` directory and it will be always applied to every loaded video file.");
        color: styleTextColor;
        textFormat: Text.MarkdownText;
    }

    Item { width: 1; height: 10 * dpiScale; }

    onClicked: (index) => {
        if (index == 0) { // Save/Apply
            let finalObj = { };
            sectionsArea.forAllCheckboxes(sectionsArea, function(cb) {
                for (const x of cb.props) {
                    if (cb.group) {
                        if (!finalObj[cb.group]) finalObj[cb.group] = { };
                        finalObj[cb.group][x] = cb.checked;
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
    buttons: [isPreset? qsTr("Save") : qsTr("Apply"), qsTr("Cancel")];
    accentButton: 0;

    function copyObj(from: var, by: var, to: var) {
        for (const key in by) {
            if (typeof by[key] === "boolean") {
                if (by[key]) {
                    to[key] = from[key];
                }
            } else {
                to[key] = { };
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
}
