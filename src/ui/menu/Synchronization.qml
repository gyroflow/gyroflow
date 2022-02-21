// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import Qt.labs.settings

import "../components/"

MenuItem {
    id: sync;
    text: qsTr("Synchronization");
    icon: "sync";
    innerItem.enabled: window.videoArea.vid.loaded && !controller.sync_in_progress;
    loader: controller.sync_in_progress;

    Settings {
        property alias initialOffset: initialOffset.value;
        property alias syncSearchSize: syncSearchSize.value;
        property alias maxSyncPoints: maxSyncPoints.value;
        property alias timePerSyncpoint: timePerSyncpoint.value;
        property alias sync_lpf: lpf.value;
        property alias syncMethod: syncMethod.currentIndex;
        property alias offsetMethod: offsetMethod.currentIndex;
        property alias showFeatures: showFeatures.checked;
        property alias showOF: showOF.checked;
        // This is a specific use case and I don't think we should remember that setting, especially that it's hidden under "Advanced"
        //property alias everyNthFrame: everyNthFrame.value; 
    }

    property alias timePerSyncpoint: timePerSyncpoint.value;
    property alias initialOffset: initialOffset.value;
    property alias syncSearchSize: syncSearchSize.value;
    property alias everyNthFrame: everyNthFrame.value;

    Button {
        text: qsTr("Auto sync");
        icon.name: "spinner"
        anchors.horizontalCenter: parent.horizontalCenter;
        enabled: controller.gyro_loaded;
        tooltip: !enabled? qsTr("No motion data loaded, cannot sync.") : "";
        function doSync() {
            const points = maxSyncPoints.value;

            const trimmed = videoArea.trimEnd - videoArea.trimStart;

            const chunks = trimmed / points;
            const start = videoArea.trimStart + (chunks / 2);
            let ranges = [];
            for (let i = 0; i < points; ++i) {
                const pos = start + (i*chunks);
                ranges.push(pos);
            }

            controller.start_autosync(ranges.join(";"), initialOffset.value * 1000, syncSearchSize.value * 1000, timePerSyncpoint.value * 1000, everyNthFrame.value, false);
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
    }

    InfoMessageSmall {
        property bool usesQuats: window.motionData.hasQuaternions && window.motionData.integrationMethod === 0 && window.motionData.filename == window.vidInfo.filename;
        show: usesQuats && controller.offsets_model.rowCount() > 0;
        text: qsTr("This file uses synced motion data, additional sync points are not needed and can make the output look worse.");
        onUsesQuatsChanged: sync.opened = !usesQuats; 
    }

    Label {
        position: Label.Left;
        text: qsTr("Rough gyro offset");

        NumberField {
            id: initialOffset;
            width: parent.width;
            height: 25 * dpiScale;
            defaultValue: 0;
            precision: 1;
            unit: qsTr("s");
        }
    }

    Label {
        position: Label.Left;
        text: qsTr("Sync search size");

        NumberField {
            id: syncSearchSize;
            width: parent.width;
            height: 25 * dpiScale;
            precision: 1;
            value: 5;
            unit: qsTr("s");
        }
    }
    Label {
        position: Label.Left;
        text: qsTr("Max sync points");

        NumberField {
            id: maxSyncPoints;
            width: parent.width;
            height: 25 * dpiScale;
            value: 3;
            from: 1;
            to: 10;
            onValueChanged: { if (value < 1) value = 1; if (value > 10) value = 10; }
        }
    }

    AdvancedSection {
        Label {
            position: Label.Left;
            text: qsTr("Analyze every n-th frame");

            NumberField {
                id: everyNthFrame;
                width: parent.width;
                height: 25 * dpiScale;
                value: 1;
                from: 1;
            }
        }
        Label {
            position: Label.Left;
            text: qsTr("Time to analyze per sync point");

            NumberField {
                id: timePerSyncpoint;
                width: parent.width;
                height: 25 * dpiScale;
                value: 1.5;
                precision: 1;
                unit: qsTr("s");
                from: 1;
            }
        }
        InfoMessageSmall {
            show: syncMethod.currentValue == "AKAZE";
            text: qsTr("The AKAZE method may be more accurate but is significantly slower than OpenCV. Use only if OpenCV doesn't produce good results"); 
        }
        Label {
            position: Label.Left;
            text: qsTr("Optical flow method");

            ComboBox {
                id: syncMethod;
                model: ["AKAZE", "OpenCV"];
                font.pixelSize: 12 * dpiScale;
                width: parent.width;
                currentIndex: 1;
                onCurrentIndexChanged: controller.sync_method = currentIndex;
            }
        }
        Label {
            text: qsTr("Offset calculation method");

            ComboBox {
                id: offsetMethod;
                model: [QT_TRANSLATE_NOOP("Popup", "Using essential matrix"), QT_TRANSLATE_NOOP("Popup", "Using visual features")];
                font.pixelSize: 12 * dpiScale;
                width: parent.width;
                onCurrentIndexChanged: controller.offset_method = currentIndex;
                tooltip: currentIndex == 0? 
                    qsTr("Calculate camera transformation matrix from optical flow to get the rotation angles of the camera.\nThen try to match these angles to gyroscope angles.")
                  : qsTr("Undistort optical flow points using gyro and candidate offset.\nThen calculate lengths of the optical flow lines.\nResulting offset is the one where lines were the shortest, meaning the video was moving the least visually.");
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
