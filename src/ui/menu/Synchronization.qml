// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import Qt.labs.settings

import "../components/"

MenuItem {
    id: sync;
    text: qsTr("Synchronization");
    iconName: "sync";
    innerItem.enabled: window.videoArea.vid.loaded && !controller.sync_in_progress;
    loader: controller.sync_in_progress;
    objectName: "synchronization";

    Settings {
        property alias processingResolution: processingResolution.currentIndex;
        property alias initialOffset: initialOffset.value;
        property alias syncSearchSize: syncSearchSize.value;
        property alias maxSyncPoints: maxSyncPoints.value;
        property alias timePerSyncpoint: timePerSyncpoint.value;
        property alias sync_lpf: lpf.value;
        property alias checkNegativeInitialOffset: checkNegativeInitialOffset.checked;
        property alias experimentalAutoSyncPoints: experimentalAutoSyncPoints.checked;
        // property alias syncMethod: syncMethod.currentIndex;
        // property alias offsetMethod: offsetMethod.currentIndex;
        // property alias poseMethod: poseMethod.currentIndex;
        property alias showFeatures: showFeatures.checked;
        property alias showOF: showOF.checked;
        // This is a specific use case and I don't think we should remember that setting, especially that it's hidden under "Advanced"
        //property alias everyNthFrame: everyNthFrame.value;
    }

    property alias timePerSyncpoint: timePerSyncpoint;
    property alias everyNthFrame: everyNthFrame;
    property alias poseMethod: poseMethod;
    property var customSyncTimestamps: [];

    function loadGyroflow(obj) {
        const o = obj.synchronization || { };
        if (o && Object.keys(o).length > 0) {
            if (o.hasOwnProperty("initial_offset"))     initialOffset.value                 = +o.initial_offset;
            if (o.hasOwnProperty("initial_offset_inv")) checkNegativeInitialOffset.checked  = !!o.initial_offset_inv;
            if (o.hasOwnProperty("search_size"))        syncSearchSize.value                = +o.search_size;
            if (o.hasOwnProperty("calc_initial_fast"))  calculateInitialOffsetFirst.checked = !!o.calc_initial_fast;
            if (o.hasOwnProperty("max_sync_points"))    maxSyncPoints.value                 = +o.max_sync_points;
            if (o.hasOwnProperty("every_nth_frame"))    everyNthFrame.value                 = +o.every_nth_frame;
            if (o.hasOwnProperty("time_per_syncpoint")) timePerSyncpoint.value              = +o.time_per_syncpoint;
            if (o.hasOwnProperty("of_method"))          syncMethod.currentIndex             = +o.of_method;
            if (o.hasOwnProperty("offset_method"))      offsetMethod.currentIndex           = +o.offset_method;
            if (o.hasOwnProperty("pose_method"))        poseMethod.currentIndex             = +o.pose_method;
            if (o.hasOwnProperty("custom_sync_timestamps")) sync.customSyncTimestamps       = o.custom_sync_timestamps;
            if (o.hasOwnProperty("auto_sync_points")) experimentalAutoSyncPoints.checked    = !!o.experimental_auto_sync_points;
            if (o.hasOwnProperty("do_autosync") && o.do_autosync) autosyncTimer.doRun = true;
        }
    }
    Timer {
        id: autosyncTimer;
        interval: 200;
        property bool doRun: false;
        running: controller.lens_loaded && controller.gyro_loaded && !window.isDialogOpened && doRun && render_queue.editing_job_id == 0;
        onTriggered: {
            doRun = false;
            if (controller.offsets_model.rowCount() == 0)
                autosync.doSync();
        }
    }
    function getSettings() {
        return {
            "initial_offset":     initialOffset.value,
            "initial_offset_inv": checkNegativeInitialOffset.checked,
            "search_size":        syncSearchSize.value,
            "calc_initial_fast":  calculateInitialOffsetFirst.checked,
            "max_sync_points":    maxSyncPoints.value,
            "every_nth_frame":    everyNthFrame.value,
            "time_per_syncpoint": timePerSyncpoint.value,
            "of_method":          syncMethod.currentIndex,
            "offset_method":      offsetMethod.currentIndex,
            "pose_method":        poseMethod.currentIndex,
            "auto_sync_points":   experimentalAutoSyncPoints.checked,
        };
    }
    function getSettingsJson() { return JSON.stringify(getSettings()); }

    Button {
        id: autosync;
        text: qsTr("Auto sync");
        iconName: "spinner"
        anchors.horizontalCenter: parent.horizontalCenter;
        enabled: controller.gyro_loaded;
        tooltip: !enabled? qsTr("No motion data loaded, cannot sync.") : "";
        function doSync() {
            const maxPoints = maxSyncPoints.value;
            let sync_points = null;

            if (experimentalAutoSyncPoints.checked) {
                sync_points = controller.get_optimal_sync_points(maxPoints);
            }
            if (!sync_points) {
                const trimmed = videoArea.trimEnd - videoArea.trimStart;
                const chunks = trimmed / maxPoints;
                const start = videoArea.trimStart + (chunks / 2);

                let ranges = [];
                for (let i = 0; i < maxPoints; ++i) {
                    const pos = start + (i*chunks);
                    ranges.push(pos);
                }
                if (sync.customSyncTimestamps.length > 0) {
                    const duration = window.videoArea.timeline.durationMs;
                    ranges = sync.customSyncTimestamps.filter(v => v <= duration).map(v => v / duration);
                }
                sync_points = ranges.join(";");
            }
            controller.start_autosync(sync_points, sync.getSettingsJson(), "synchronize");
        }
        onClicked: {
            if (!controller.lens_loaded) {
                messageBox(Modal.Warning, qsTr("Lens profile is not loaded, synchronization will most likely give wrong results. Are you sure you want to continue?"), [
                    { text: qsTr("Yes"), clicked: function() {
                        doSync();
                    }},
                    { text: qsTr("No"), accent: true },
                ]);
            } else {
                doSync();
            }
        }

        CheckBox {
            id: experimentalAutoSyncPoints;
            anchors.left: autosync.right;
            anchors.leftMargin: 5 * dpiScale;
            anchors.verticalCenter: parent.verticalCenter;
            contentItem.visible: false;
            scale: 0.7;
            tooltip: qsTr("Experimental automatic sync point selection.");
        }
    }

    InfoMessageSmall {
        property bool usesQuats: window.motionData.hasQuaternions && window.motionData.integrationMethod === 0 && window.motionData.filename == window.vidInfo.filename;
        show: usesQuats && controller.offsets_model.rowCount() > 0;
        text: qsTr("This file uses synced motion data, additional sync points are not needed and can make the output look worse.");
        onUsesQuatsChanged: sync.opened = !usesQuats;
    }

    Label {
        position: Label.LeftPosition;
        text: qsTr("Rough gyro offset");

        NumberField {
            id: initialOffset;
            width: parent.width - checkNegativeInitialOffset.width;
            height: 25 * dpiScale;
            defaultValue: 0;
            precision: 1;
            unit: qsTr("s");
        }
        CheckBox {
            id: checkNegativeInitialOffset;
            anchors.left: initialOffset.right;
            anchors.leftMargin: 5 * dpiScale;
            anchors.verticalCenter: parent.verticalCenter;
            contentItem.visible: false;
            scale: 0.7;
            tooltip: qsTr("Analyze both positive and negative offset.\nThis doubles the calculation time, so check this only for the initial point and uncheck once you know the offset.");
        }
    }

    Label {
        position: Label.LeftPosition;
        text: qsTr("Sync search size");

        NumberField {
            id: syncSearchSize;
            width: parent.width - (calculateInitialOffsetFirst.visible? calculateInitialOffsetFirst.width : 0);
            height: 25 * dpiScale;
            precision: 1;
            value: 5;
            defaultValue: 5;
            unit: qsTr("s");
            onValueChanged: if (calculateInitialOffsetFirst.visible) calculateInitialOffsetFirst.checked = value > 10;
        }
        CheckBox {
            id: calculateInitialOffsetFirst;
            anchors.left: syncSearchSize.right;
            anchors.leftMargin: 5 * dpiScale;
            anchors.verticalCenter: parent.verticalCenter;
            contentItem.visible: false;
            scale: 0.7;
            visible: offsetMethod.currentIndex > 0;
            tooltip: qsTr("Calculate initial offset first (using essential matrix method), then refine using slower but more accurate rs-sync method.");
        }
    }
    Label {
        position: Label.LeftPosition;
        text: qsTr("Max sync points");

        NumberField {
            id: maxSyncPoints;
            width: parent.width;
            height: 25 * dpiScale;
            value: 3;
            from: 1;
            to: 30;
            onValueChanged: { if (value < 1) value = 1; if (value > 500) value = 500; }
        }
    }

    AdvancedSection {
        Label {
            position: Label.LeftPosition;
            text: qsTr("Analyze every n-th frame");

            NumberField {
                id: everyNthFrame;
                width: parent.width;
                height: 25 * dpiScale;
                value: 1;
                defaultValue: 1;
                from: 1;
            }
        }
        Label {
            position: Label.LeftPosition;
            text: qsTr("Time to analyze per sync point");

            NumberField {
                id: timePerSyncpoint;
                width: parent.width;
                height: 25 * dpiScale;
                value: 1.5;
                defaultValue: 1.5;
                precision: 2;
                unit: qsTr("s");
                from: 0.01;
            }
        }
        Label {
            position: Label.LeftPosition;
            text: qsTr("Processing resolution");
            ComboBox {
                id: processingResolution;
                model: [QT_TRANSLATE_NOOP("Popup", "Full"), "4k", "1080p", "720p", "480p"];
                font.pixelSize: 12 * dpiScale;
                width: parent.width;
                currentIndex: 3;
                onCurrentIndexChanged: {
                    let target_height = -1; // Full
                    switch (currentIndex) {
                        case 1: target_height = 2160; break;
                        case 2: target_height = 1080; break;
                        case 3: target_height = 720; break;
                        case 4: target_height = 480; break;
                    }

                    controller.set_processing_resolution(target_height);
                }
            }
        }
        InfoMessageSmall {
            show: syncMethod.currentValue == "AKAZE";
            text: qsTr("The AKAZE method may be more accurate but is significantly slower than OpenCV. Use only if OpenCV doesn't produce good results");
        }
        Label {
            position: Label.LeftPosition;
            text: qsTr("Optical flow method");

            ComboBox {
                id: syncMethod;
                model: ["AKAZE", "OpenCV (PyrLK)", "OpenCV (DIS)"];
                font.pixelSize: 12 * dpiScale;
                width: parent.width;
                currentIndex: 2;
                onCurrentIndexChanged: controller.set_of_method(currentIndex);
                Component.onCompleted: currentIndexChanged();
            }
        }
        Label {
            text: qsTr("Pose method");
            position: Label.LeftPosition;

            ComboBox {
                id: poseMethod;
                model: ["findEssentialMat", "Almeida", "EightPoint", "findHomography"];
                font.pixelSize: 12 * dpiScale;
                width: parent.width;
                currentIndex: 0;
                onCurrentIndexChanged: controller.set_of_method(syncMethod.currentIndex);
            }
        }
        Label {
            text: qsTr("Offset method");
            position: Label.LeftPosition;

            ComboBox {
                id: offsetMethod;
                model: [QT_TRANSLATE_NOOP("Popup", "Essential matrix"), QT_TRANSLATE_NOOP("Popup", "Visual features"), QT_TRANSLATE_NOOP("Popup", "rs-sync")];
                font.pixelSize: 12 * dpiScale;
                width: parent.width;
                currentIndex: 2;
                property var tooltips: ([
                    qsTr("Calculate camera transformation matrix from optical flow to get the rotation angles of the camera.\nThen try to match these angles to gyroscope angles."),
                    qsTr("Undistort optical flow points using gyro and candidate offset.\nThen calculate lengths of the optical flow lines.\nResulting offset is the one where lines were the shortest, meaning the video was moving the least visually."),
                    qsTr("Rolling shutter video to gyro synchronization algorithm.\nMake sure you have proper rolling shutter value set before syncing.")
                ]);
                tooltip: tooltips[currentIndex];
            }
        }
        CheckBoxWithContent {
            id: lpfcb;
            text: qsTr("Low pass filter");
            onCheckedChanged: controller.set_sync_lpf(checked? lpf.value : 0);

            NumberField {
                id: lpf;
                unit: qsTr("Hz");
                precision: 2;
                value: 0;
                defaultValue: 0;
                from: 0;
                width: parent.width;
                onValueChanged: {
                    controller.set_sync_lpf(lpfcb.checked? lpf.value : 0);
                }
            }
        }
        CheckBox {
            id: showFeatures;
            text: qsTr("Show detected features");
            checked: true;
            onCheckedChanged: controller.show_detected_features = checked;
        }
        CheckBox {
            id: showOF;
            text: qsTr("Show optical flow");
            checked: true;
            onCheckedChanged: controller.show_optical_flow = checked;
        }
    }
}
