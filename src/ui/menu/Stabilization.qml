// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import Qt.labs.settings

import "../components/"

MenuItem {
    id: root;
    text: qsTr("Stabilization");
    icon: "gyroflow";
    innerItem.enabled: window.videoArea.vid.loaded;

    property alias fovSlider: fov;
    property alias maxValues: maxValues;

    Settings {
        id: settings;
        property alias smoothingMethod: smoothingMethod.currentIndex;
        property alias croppingMode: croppingMode.currentIndex;
        property alias adaptiveZoom: adaptiveZoom.value;
    }

    function setFrameReadoutTime(v) {
        shutter.value = Math.abs(v);
        shutterCb.checked = Math.abs(v) > 0;
        bottomToTop.checked = v < 0;
    }

    function setSmoothingParam(name, value) {
        settings.setValue("smoothing-" + smoothingMethod.currentIndex + "-" + name, value);
        controller.set_smoothing_param(name, value);
    }
    function getSmoothingParam(name, defaultValue) {
        return settings.value("smoothing-" + smoothingMethod.currentIndex + "-" + name, defaultValue);
    }

    Connections {
        target: controller;
        function onCompute_progress(id, progress) {
            if (progress >= 1) {
                const min_fov = controller.get_min_fov();
                const max_angles = controller.get_smoothing_max_angles();
                maxValues.maxPitch = max_angles[0];
                maxValues.maxYaw   = max_angles[1];
                maxValues.maxRoll  = max_angles[2];
                maxValues.maxZoom  = min_fov > 0.0001? (100 / min_fov) : min_fov;
                const status = controller.get_smoothing_status();
                // Clear current params
                for (let i = smoothingStatus.children.length; i > 0; --i) {
                    smoothingStatus.children[i - 1].destroy();
                }

                if (status.length > 0) {
                    let qml = "import QtQuick; import '../components/'; Column { width: parent.width; ";
                    for (const x of status) {
                        // TODO: figure out a better way than constructing a string
                        switch (x.type) {
                            case 'Label':
                                let text = qsTranslate("Stabilization", x.text).replace(/\n/g, "<br>");
                                if (x.text_args) {
                                    for (const arg of x.text_args) {
                                        text = text.arg(arg);
                                    }
                                }
                                qml += `BasicText {
                                    width: parent.width;
                                    wrapMode: Text.WordWrap;
                                    textFormat: Text.StyledText;
                                    text: "${text}"
                                }`;
                            break;
                            case 'QML': qml += x.custom_qml; break;
                        }
                    }
                    qml += "}";

                    Qt.createQmlObject(qml, smoothingStatus);
                }
            }
        }
    }
    
    Component.onCompleted: {
        QT_TRANSLATE_NOOP("Popup", "No smoothing");
        QT_TRANSLATE_NOOP("Popup", "Plain 3D");
        QT_TRANSLATE_NOOP("Popup", "Velocity dampened"),
        QT_TRANSLATE_NOOP("Popup", "Velocity dampened per axis"),
        // QT_TRANSLATE_NOOP("Popup", "Velocity dampened 2"),
        QT_TRANSLATE_NOOP("Popup", "Fixed camera");
        QT_TRANSLATE_NOOP("Popup", "Lock horizon"),

        QT_TRANSLATE_NOOP("Stabilization", "Pitch smoothness");
        QT_TRANSLATE_NOOP("Stabilization", "Yaw smoothness");
        QT_TRANSLATE_NOOP("Stabilization", "Roll smoothness");
        QT_TRANSLATE_NOOP("Stabilization", "Smoothness");
        QT_TRANSLATE_NOOP("Stabilization", "Yaw angle correction");
        QT_TRANSLATE_NOOP("Stabilization", "Pitch angle correction");
        QT_TRANSLATE_NOOP("Stabilization", "Roll angle correction");
        QT_TRANSLATE_NOOP("Stabilization", "Yaw angle");
        QT_TRANSLATE_NOOP("Stabilization", "Pitch angle");
        QT_TRANSLATE_NOOP("Stabilization", "Roll angle");
        // QT_TRANSLATE_NOOP("Stabilization", "Pitch velocity dampening");
        // QT_TRANSLATE_NOOP("Stabilization", "Yaw velocity dampening");
        // QT_TRANSLATE_NOOP("Stabilization", "Roll velocity dampening");
        QT_TRANSLATE_NOOP("Stabilization", "Max rotation:\nPitch: %1, Yaw: %2, Roll: %3.\nModify dampening settings until you get the desired values (recommended around 6 on all axes).");
        QT_TRANSLATE_NOOP("Stabilization", "Max rotation:\nPitch: %1, Yaw: %2, Roll: %3.\nModify velocity factor until you get the desired values (recommended less than 20).");
        QT_TRANSLATE_NOOP("Stabilization", "Modify dampening settings until you get the desired values (recommended around 6 on all axes).");
        QT_TRANSLATE_NOOP("Stabilization", "Modify velocity factor until you get the desired values (recommended less than 20).");
        QT_TRANSLATE_NOOP("Stabilization", "Smoothness at high velocity");
        QT_TRANSLATE_NOOP("Stabilization", "Velocity factor");
        QT_TRANSLATE_NOOP("Stabilization", "Smoothness multiplier");
        QT_TRANSLATE_NOOP("Stabilization", "Responsiveness");
    }

    Connections {
        target: controller;
        function onTelemetry_loaded(is_main_video, filename, camera, imu_orientation, contains_gyro, contains_quats, frame_readout_time, camera_id_json) {
            root.setFrameReadoutTime(frame_readout_time);
        }
        function onRolling_shutter_estimated(rolling_shutter) {
            root.setFrameReadoutTime(rolling_shutter);
        }
    }

    InfoMessageSmall {
        id: fovWarning;
        show: fov.value > 1.0 && croppingMode.currentIndex > 0;
        text: qsTr("FOV is greater than 1.0, you may see black borders"); 
    }

    Label {
        position: Label.Left;
        text: qsTr("FOV");
        SliderWithField {
            id: fov;
            from: 0.1;
            to: 3;
            value: 1.0;
            width: parent.width;
            onValueChanged: controller.fov = value;
        }
    }

    ComboBox {
        id: smoothingMethod;
        model: smoothingAlgorithms;
        font.pixelSize: 12 * dpiScale;
        width: parent.width;
        currentIndex: 2;
        Component.onCompleted: currentIndexChanged();
        onCurrentIndexChanged: {
            // Clear current params
            for (let i = smoothingOptions.children.length; i > 0; --i) {
                smoothingOptions.children[i - 1].destroy();
            }

            const opt_json = controller.set_smoothing_method(currentIndex);
            if (opt_json.length > 0) {
                let qml = "import QtQuick; import '../components/'; Column { width: parent.width; ";
                for (const x of opt_json) {
                    // TODO: figure out a better way than constructing a string
                    switch (x.type) {
                        case 'Slider': 
                        case 'SliderWithField': 
                        case 'NumberField': 
                            qml += `Label {
                                width: parent.width;
                                spacing: 2 * dpiScale;
                                text: qsTranslate("Stabilization", "${x.description}")
                                ${x.type} {
                                    width: parent.width;
                                    from: ${x.from};
                                    to: ${x.to};
                                    value: root.getSmoothingParam("${x.name}", ${x.value});
                                    defaultValue: ${x.value};
                                    unit: qsTranslate("Stabilization", "${x.unit}");
                                    //live: false;
                                    precision: ${x.precision} || 2;
                                    onValueChanged: root.setSmoothingParam("${x.name}", value);
                                }
                            }`;
                        break;
                        case 'QML': qml += x.custom_qml; break;
                    }
                }
                qml += "}";

                Qt.createQmlObject(qml, smoothingOptions);
            }
        }
    }
    Column {
        id: smoothingOptions;
        x: 5 * dpiScale;
        width: parent.width - x;
        visible: children.length > 0;
    }
    Column {
        id: smoothingStatus;
        x: 5 * dpiScale;
        width: parent.width - x;
        visible: children.length > 0;
    }
    
    InfoMessageSmall {
        id: maxValues;
        property real maxPitch: 0;
        property real maxYaw: 0;
        property real maxRoll: 0;
        property real maxZoom: 0;
        show: true;
        //color: styleBackground;
        color: "transparent";
        border.width: 0 * dpiScale;
        border.color: styleVideoBorderColor;
        //t.x: 10 * dpiScale;
        t.x: 0;
        //height: t.height + 20 * dpiScale;
        height: t.height + 5 * dpiScale;
        t.color: styleTextColor;
        t.horizontalAlignment: Text.AlignLeft;
        text: qsTr("Max rotation: Pitch: %1, Yaw: %2, Roll: %3")
                .arg("<b>" + maxPitch.toFixed(1) + "°</b>")
                .arg("<b>" + maxYaw  .toFixed(1) + "°</b>")
                .arg("<b>" + maxRoll .toFixed(1) + "°</b>")
              + "<br>"
              + qsTr("Max zoom: %1").arg("<b>" + maxZoom.toFixed(1) + "%</b>"); 
    }

    ComboBox {
        id: croppingMode;
        font.pixelSize: 12 * dpiScale;
        width: parent.width;
        model: [QT_TRANSLATE_NOOP("Popup", "No cropping"), QT_TRANSLATE_NOOP("Popup", "Dynamic cropping"), QT_TRANSLATE_NOOP("Popup", "Static crop")];
        Component.onCompleted: currentIndexChanged();
        onCurrentIndexChanged: {
            switch (currentIndex) {
                case 0: controller.adaptive_zoom = 0.0; break;
                case 1: controller.adaptive_zoom = adaptiveZoom.value; break;
                case 2: controller.adaptive_zoom = -1.0; break;
            }
        }
    }
    Label {
        text: qsTr("Smoothing window");
        visible: croppingMode.currentIndex == 1;
        SliderWithField {
            id: adaptiveZoom;
            value: 4;
            from: 0.1;
            to: 15;
            unit: qsTr("s");
            width: parent.width;
            onValueChanged: controller.adaptive_zoom = value;
        }
    }

    CheckBoxWithContent {
        id: shutterCb;
        text: qsTr("Rolling shutter correction");
        cb.onCheckedChanged: {
            controller.frame_readout_time = cb.checked? (bottomToTop.checked? -shutter.value : shutter.value) : 0.0;
        }

        Label {
            text: qsTr("Frame readout time");
            SliderWithField {
                id: shutter;
                to: 1000 / Math.max(1, window.videoArea.vid.frameRate);
                width: parent.width;
                unit: qsTr("ms");
                precision: 2;
                onValueChanged: controller.frame_readout_time = bottomToTop.checked? -value : value;
            }
            CheckBox {
                id: bottomToTop;
                anchors.right: parent.right;
                anchors.top: parent.top;
                anchors.topMargin: -30 * dpiScale;
                anchors.rightMargin: -10 * dpiScale;
                contentItem.visible: false;
                scale: 0.7;
                tooltip: qsTr("Bottom to top")
                onCheckedChanged: controller.frame_readout_time = bottomToTop.checked? -shutter.value : shutter.value;
            }
        }
    }
}
