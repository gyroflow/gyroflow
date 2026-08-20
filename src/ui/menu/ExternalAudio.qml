// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rodrigo Sclosa

import QtQuick

import "../components/"

/**
 * External audio panel.
 *
 * Imports a separately recorded track (DJI Mic and similar), aligns it to the
 * video - manually or by correlating propeller vibration - and embeds it in the
 * exported file preserving the source format.
 *
 * The waveform is shown as a timeline lane, below the gyro chart.
 */
MenuItem {
    id: root;
    text: qsTr("External audio");
    iconName: "sound";
    objectName: "externalaudio";
    // Same pattern as Synchronization.qml: the panel shows a spinner and locks
    // while a long operation runs on a worker thread.
    readonly property bool busy: controller.external_audio_loading || controller.external_audio_syncing;
    innerItem.enabled: window.videoArea.vid.loaded && !root.busy;
    loader: root.busy;

    /// Whether the source format is preserved losslessly on export.
    property bool audioPreserveFormat: true;
    /// Whether the blade band is detected from the signal itself.
    property bool audioAutoBand: true;
    /// Fixed band, used when audioAutoBand is off or as a fallback.
    property real audioBandLo: 150;
    property real audioBandHi: 900;
    /// High-pass cutoff applied to the gyro, to remove intentional movement.
    property real audioHighpass: 30;

    /// Whether a track is loaded. Controls the visibility of almost everything here.
    readonly property bool hasAudio: !!audioInfo.text;

    /// Translated name of a sample format. The core returns a stable key so the
    /// label can be localized here instead of shipping English to every language.
    function formatLabel(key: string): string {
        switch (key) {
            case "f32": return qsTr("32-bit float");
            case "s32": return qsTr("32-bit int");
            case "s24": return qsTr("24-bit int");
            case "s16": return qsTr("16-bit int");
            case "u8":  return qsTr("8-bit");
            default:    return qsTr("compressed");
        }
    }

    /// Detected format, as one translatable sentence rather than concatenated
    /// fragments - word order differs between languages.
    function formatSummary(json: string): string {
        if (!json) return "";
        const info = JSON.parse(json);
        return qsTr("%1 Hz, %2 channels, %3")
               .arg(info.sample_rate)
               .arg(info.channels)
               .arg(root.formatLabel(info.source_format));
    }

    function getAudioWaveform(): var {
        return window.videoArea && window.videoArea.timeline
             ? window.videoArea.timeline.getAudioWaveform()
             : null;
    }

    /// Track state stored in the `.gyroflow` file.
    function getSettings(): var {
        const path = controller.get_external_audio_url();
        if (!path) return { };
        return {
            "path":                     path,
            "sample_rate":              controller.get_external_audio_sample_rate(),
            "offset_seconds":           audioOffset.value / 1000.0,
            "preserve_original_format": root.audioPreserveFormat,
            // Detection parameters, so that drones with an unusual blade
            // frequency can be adjusted without recompiling.
            "auto_band":                root.audioAutoBand,
            "band_lo_hz":               root.audioBandLo,
            "band_hi_hz":               root.audioBandHi,
            "highpass_hz":              root.audioHighpass
        };
    }

    function loadGyroflow(obj: var): void {
        // Files saved before this feature simply don't have this key, and keep
        // opening normally.
        const a = obj.audio_sync || { };
        if (!a || !a.hasOwnProperty("path") || !a.path) return;

        if (a.hasOwnProperty("preserve_original_format")) root.audioPreserveFormat = !!a.preserve_original_format;
        if (a.hasOwnProperty("auto_band"))   root.audioAutoBand = !!a.auto_band;
        if (a.hasOwnProperty("band_lo_hz"))  root.audioBandLo   = +a.band_lo_hz;
        if (a.hasOwnProperty("band_hi_hz"))  root.audioBandHi   = +a.band_hi_hz;
        if (a.hasOwnProperty("highpass_hz")) root.audioHighpass = +a.highpass_hz;

        // The offset is applied after decoding: reloading the file re-detects
        // channels, bit depth and float format, and only then does repositioning
        // the waveform make sense.
        // Decoding happens on a worker thread; the offset travels with the
        // request and comes back in external_audio_imported.
        const pendingOffset = a.hasOwnProperty("offset_seconds") ? +a.offset_seconds : 0.0;
        controller.import_external_audio_url(a.path, pendingOffset * 1000.0);
    }

    FileDialog {
        id: audioFileDialog;
        property var extensions: ["wav", "m4a", "mp3", "flac", "aac", "mp4"];
        title: qsTr("Choose an audio file");
        nameFilters: [qsTr("Audio files") + " (*.wav *.m4a *.mp3 *.flac *.aac *.mp4)"];
        type: "audio";
        onAccepted: controller.import_external_audio(audioFileDialog.selectedFile);
    }

    Button {
        text: root.hasAudio? qsTr("Replace audio file") : qsTr("Import external audio");
        iconName: "sound";
        width: parent.width;
        onClicked: audioFileDialog.open2();
    }

    // Detected format. This is where the user confirms that 32-bit float was
    // recognized as such.
    BasicText {
        id: audioInfo;
        width: parent.width;
        wrapMode: Text.WordWrap;
        leftPadding: 0;
        text: root.formatSummary(controller.get_external_audio_info());
        visible: !!text;

        Connections {
            target: controller;

            // Decoding finished: fill the lane and put the project offset on the
            // slider.
            function onExternal_audio_imported(ok: bool, offset_ms: real): void {
                if (!ok) return;
                controller.refresh_audio_waveform(root.getAudioWaveform());
                audioOffset.value = offset_ms;
            }

            function onExternal_audio_changed() {
                audioInfo.text = root.formatSummary(controller.get_external_audio_info());
                audioPath.text = controller.get_external_audio_path();
                formatBadge.refresh();

                // Loading another video discards the track on the Rust side; the
                // lane and the slider must follow, otherwise leftovers from the
                // previous clip remain.
                if (!audioInfo.text) {
                    const waveform = root.getAudioWaveform();
                    if (waveform) waveform.clear();
                    audioOffset.value = 0;
                    autoSyncResult.text = "";
                }
            }
        }
    }
    BasicText {
        id: audioPath;
        width: parent.width;
        wrapMode: Text.WrapAnywhere;
        leftPadding: 0;
        font.pixelSize: 11 * dpiScale;
        opacity: 0.6;
        text: controller.get_external_audio_path();
        visible: !!text;
    }

    // ---- Auto-sync ----
    Button {
        text: qsTr("Auto-sync audio");
        iconName: "sync";
        width: parent.width;
        visible: root.hasAudio;
        enabled: controller.gyro_loaded;
        tooltip: qsTr("Aligns the audio by correlating propeller vibration picked up by the microphone with the vibration read by the gyroscope.");
        onClicked: controller.auto_sync_external_audio(root.audioAutoBand, root.audioBandLo, root.audioBandHi, root.audioHighpass);

        Connections {
            target: controller;
            // The correlation runs on a worker thread; this is where it lands.
            function onExternal_audio_synced(result: string): void {
                if (!result) {
                    autoSyncResult.text = qsTr("Not enough data to sync");
                    autoSyncResult.isWeak = true;
                    return;
                }
                const r = JSON.parse(result);
                audioOffset.value = r.offset_seconds * 1000.0;

                // Low confidence almost always means the blade vibration didn't reach
                // the gyro (gimbal isolating it) or the mic was far from the drone.
                autoSyncResult.isWeak = r.confidence < 0.3;

                // With cameras that only provide quaternions (DJI O4P and similar)
                // there is no propeller vibration in the gyro signal, and the
                // alignment is done by start of movement - less accurate, so it is
                // worth telling which one was used.
                const method = r.method === "onset"
                             ? qsTr("aligned by start of movement")
                             : qsTr("aligned by propeller vibration");

                // One sentence per case instead of concatenated fragments: word order
                // and punctuation differ between languages.
                const confidence = (r.confidence * 100).toFixed(0);
                autoSyncResult.text = autoSyncResult.isWeak
                    ? qsTr("Confidence: %1% — %2 — weak match, check manually").arg(confidence).arg(method)
                    : qsTr("Confidence: %1% — %2").arg(confidence).arg(method);
            }
        }
    }
    BasicText {
        id: autoSyncResult;
        property bool isWeak: false;
        width: parent.width;
        leftPadding: 0;
        wrapMode: Text.WordWrap;
        font.pixelSize: 11 * dpiScale;
        color: isWeak? "#cc8866" : styleTextColor;
        visible: !!text && root.hasAudio;
    }

    // ---- Manual offset ----
    // Dragging only re-maps the waveform drawing position: no audio is decoded again.
    Label {
        position: Label.LeftPosition;
        text: qsTr("Offset");
        visible: root.hasAudio;

        SliderWithField {
            id: audioOffset;
            width: parent.width;
            from: -30000;
            to: 30000;
            value: 0;
            defaultValue: 0;
            unit: qsTr("ms");
            precision: 1;
            live: true;
            onValueChanged: {
                const seconds = value / 1000.0;
                const waveform = root.getAudioWaveform();
                if (waveform) waveform.offsetSeconds = seconds;
                // The lane redraws itself; the controller stores the value
                // because that is what the export reads from.
                controller.set_external_audio_offset(seconds);
            }
        }
    }

    // The same offset in frames, which is how the value is compared against an
    // editor's timeline.
    BasicText {
        width: parent.width;
        leftPadding: 0;
        visible: root.hasAudio;
        opacity: 0.7;
        font.pixelSize: 11 * dpiScale;
        text: {
            const fps = controller.get_scaled_fps();
            if (!fps) return "";
            const frames = (audioOffset.value / 1000.0) * fps;
            return qsTr("%1 frames @ %2 fps").arg(frames.toFixed(2)).arg(fps.toFixed(3));
        }
    }

    // ---- Format preservation ----
    // Any loss of precision must be visible BEFORE the export, never a surprise
    // in the final file.
    Rectangle {
        id: formatBadge;
        width: parent.width;
        height: badgeText.height + 12 * dpiScale;
        visible: root.hasAudio && !!badgeText.text;
        radius: 4 * dpiScale;
        color: formatBadge.isMismatch? "#40cc6666" : "#4066cc66";

        property bool isMismatch: false;

        function refresh(): void {
            // The extension decides: it is the video container that determines
            // whether the audio fits. It comes from the filename chosen in the
            // export panel.
            const filename = window.outputFile? window.outputFile.filename : "";
            const dot = filename.lastIndexOf(".");
            const ext = dot >= 0? filename.substring(dot + 1) : "";
            if (!ext) { badgeText.text = ""; return; }

            const raw = controller.get_external_audio_format_status(ext);
            if (!raw) { badgeText.text = ""; return; }

            const s = JSON.parse(raw);
            formatBadge.isMismatch = s.status === "mismatch";
            switch (s.status) {
                case "preserved":
                    badgeText.text = qsTr("Audio: %1 preserved (%2)").arg(root.formatLabel(s.source_format)).arg(s.codec);
                    break;
                case "mismatch":
                    // .mov is not a control of its own: the container comes from the
                    // video codec, so name the codecs that produce it.
                    badgeText.text = s.suggested_extension === "mov"
                        ? qsTr("Audio: %1 does not fit in .%2. Pick ProRes, DNxHD or CineForm as the video codec to get a .mov, otherwise the audio will be converted.")
                          .arg(root.formatLabel(s.source_format)).arg(s.extension)
                        : qsTr("Audio: %1 does not fit in .%2. Switch the output to .%3 or the audio will be converted.")
                          .arg(root.formatLabel(s.source_format)).arg(s.extension).arg(s.suggested_extension);
                    break;
                case "downgrade":
                    badgeText.text = qsTr("Audio: will be converted to %1").arg(s.codec);
                    break;
            }
        }

        BasicText {
            id: badgeText;
            anchors.centerIn: parent;
            width: parent.width - 12 * dpiScale;
            wrapMode: Text.WordWrap;
            font.pixelSize: 11 * dpiScale;
        }

        // The container can change after the audio is imported (changing the
        // video codec changes the extension), and the badge has to follow.
        Connections {
            target: window.outputFile;
            ignoreUnknownSignals: true;
            function onFilenameChanged() { formatBadge.refresh(); }
        }
    }

    CheckBox {
        text: qsTr("Preserve original audio format");
        checked: root.audioPreserveFormat;
        visible: root.hasAudio;
        tooltip: qsTr("Keeps the original bit depth and sample rate. 32-bit float stays 32-bit float, with no silent conversion.");
        // Turning this off is an explicit choice: while on (the default) the
        // audio is never converted without warning.
        onCheckedChanged: {
            root.audioPreserveFormat = checked;
            controller.set_external_audio_preserve_format(checked);
            formatBadge.refresh();
        }
    }

    // ---- Detection parameters ----
    AdvancedSection {
        visible: root.hasAudio;

        CheckBoxWithContent {
            id: autoBandCb;
            text: qsTr("Detect propeller band automatically");
            checked: root.audioAutoBand;
            onCheckedChanged: root.audioAutoBand = checked;

            Label {
                position: Label.LeftPosition;
                text: qsTr("Band");
                visible: !autoBandCb.checked;

                Row {
                    spacing: 5 * dpiScale;
                    NumberField {
                        value: root.audioBandLo;
                        unit: qsTr("Hz");
                        width: 60 * dpiScale;
                        onValueChanged: root.audioBandLo = value;
                    }
                    BasicText { text: "—"; anchors.verticalCenter: parent.verticalCenter; }
                    NumberField {
                        value: root.audioBandHi;
                        unit: qsTr("Hz");
                        width: 60 * dpiScale;
                        onValueChanged: root.audioBandHi = value;
                    }
                }
            }
        }

        // Removes intentional movement from the gyro signal, leaving only vibration.
        Label {
            position: Label.LeftPosition;
            text: qsTr("Gyro high-pass");

            NumberField {
                value: root.audioHighpass;
                unit: qsTr("Hz");
                width: 60 * dpiScale;
                onValueChanged: root.audioHighpass = value;
            }
        }
    }

    Button {
        text: qsTr("Remove audio");
        width: parent.width;
        visible: root.hasAudio;
        onClicked: {
            controller.clear_external_audio(root.getAudioWaveform());
            audioOffset.value = 0;
            autoSyncResult.text = "";
        }
    }
}
