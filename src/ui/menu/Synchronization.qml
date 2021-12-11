import QtQuick 2.15
import Qt.labs.settings 1.0

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
        property alias showFeatures: showFeatures.checked;
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

            controller.start_autosync(ranges.join(";"), initialOffset.value, syncSearchSize.value * 1000, timePerSyncpoint.value * 1000, everyNthFrame.value, window.videoArea.vid.rotation);
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
        property bool usesQuats: window.motionData.hasQuaternions && window.motionData.integrationMethod === 0;
        show: usesQuats && controller.offsets_model.rowCount() > 0;
        text: qsTr("This file uses synced motion data, additional sync points are not needed and can make the output look worse.");
        onUsesQuatsChanged: if (usesQuats) sync.opened = false; 
    }

    Label {
        position: Label.Left;
        text: qsTr("Rough gyro offset");

        NumberField {
            id: initialOffset;
            width: parent.width;
            height: 25 * dpiScale;
            precision: 1;
            unit: "s";
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
            unit: "s";
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
        }
    }

    LinkButton {
        text: qsTr("Advanced");
        anchors.horizontalCenter: parent.horizontalCenter;
        onClicked: advanced.opened = !advanced.opened;
    }
    Column {
        spacing: parent.spacing;
        id: advanced;
        property bool opened: false;
        width: parent.width;
        visible: opacity > 0;
        opacity: opened? 1 : 0;
        height: opened? implicitHeight : -10 * dpiScale;
        Ease on opacity { }
        Ease on height { id: anim; }
        onOpenedChanged: {
            anim.enabled = true;
            timer.start();
        }
        Timer {
            id: timer;
            interval: 700;
            onTriggered: anim.enabled = false;
        }

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
                unit: "s";
                from: 1;
            }
        }
        Label {
            position: Label.Left;
            text: qsTr("Method");

            ComboBox {
                id: syncMethod;
                model: ["AKAZE", "OpenCV"];
                font.pixelSize: 12 * dpiScale;
                width: parent.width;
                currentIndex: 1;
                onCurrentIndexChanged: {
                    controller.sync_method = currentIndex;
                }
            }
        }
        CheckBoxWithContent {
            text: qsTr("Low pass filter");
            onCheckedChanged: controller.set_sync_lpf(checked? lpf.value : 0);

            NumberField {
                id: lpf;
                unit: "Hz";
                precision: 2;
                value: 0;
                from: 0;
                width: parent.width;
                onValueChanged: {
                    controller.set_sync_lpf(value);
                }
            }
        }
        CheckBox {
            id: showFeatures;
            text: qsTr("Show detected features");
            checked: true;
            onCheckedChanged: controller.show_detected_features = checked;
        }
    }
}
