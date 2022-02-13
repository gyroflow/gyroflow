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

    function loadGyroflow(obj) {
        const stab = obj.stabilization || { };
        if (stab) {
            fov.value = +stab.fov;
            const methodIndex = smoothingAlgorithms.indexOf(stab.method);
            if (methodIndex > -1) {
                smoothingMethod.currentIndex = methodIndex;
            }
            if (stab.smoothing_params) {
                Qt.callLater(function() {
                    for (const x of stab.smoothing_params) {
                        if (smoothingOptions.children.length > 0) {
                            for (const y in smoothingOptions.children[0].children) {
                                const el = smoothingOptions.children[0].children[y];
                                if (el && el.inner && el.inner.children.length > 0) {
                                    const slider = el.inner.children[0];
                                    if (slider.objectName == "param-" + x.name) {
                                        console.log("Setting param", x.name, x.value);
                                        slider.value = x.value;
                                    }
                                }
                            }
                        }
                    }
                });
            }
            setFrameReadoutTime(+stab.frame_readout_time);

            const az = +stab.adaptive_zoom_window;
            if (az < -0.9) {
                croppingMode.currentIndex = 2; // Static crop
            } else if (az > 0) {
                croppingMode.currentIndex = 1; // Dynamic cropping
                adaptiveZoom.value = az;
            } else {
                croppingMode.currentIndex = 0; // No cropping
            }        
        }
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
    function getParamElement(name) {
        function traverseChildren(node) {
            for (let i = node.children.length; i > 0; --i) {
                const child = node.children[i - 1];
                if (child) {
                    if (child.objectName == ("param-" + name)) {
                        return child;
                    }
                    const found = traverseChildren(child);
                    if (found !== null) return found;
                }
            }
            return null;
        }
        return traverseChildren(smoothingOptions);
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
        QT_TRANSLATE_NOOP("Popup", "Default"),
        QT_TRANSLATE_NOOP("Popup", "Plain 3D");
        QT_TRANSLATE_NOOP("Popup", "Velocity dampened"),
        QT_TRANSLATE_NOOP("Popup", "Velocity dampened per axis"),
        QT_TRANSLATE_NOOP("Popup", "Velocity dampened (advanced)"),
        // QT_TRANSLATE_NOOP("Popup", "Velocity dampened 2"),
        QT_TRANSLATE_NOOP("Popup", "Fixed camera");
        // QT_TRANSLATE_NOOP("Popup", "Lock horizon"),

        QT_TRANSLATE_NOOP("Stabilization", "Pitch smoothness");
        QT_TRANSLATE_NOOP("Stabilization", "Yaw smoothness");
        QT_TRANSLATE_NOOP("Stabilization", "Roll smoothness");
        QT_TRANSLATE_NOOP("Stabilization", "Smoothness");
        QT_TRANSLATE_NOOP("Stabilization", "Per axis");
        QT_TRANSLATE_NOOP("Stabilization", "Max smoothness");
        QT_TRANSLATE_NOOP("Stabilization", "Yaw angle correction");
        QT_TRANSLATE_NOOP("Stabilization", "Pitch angle correction");
        QT_TRANSLATE_NOOP("Stabilization", "Roll angle correction");
        QT_TRANSLATE_NOOP("Stabilization", "Requires accurate orientation determination. Try with Complementary, Mahony, or Madgwick integration method.");
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
            defaultValue: 1.0;
            width: parent.width;
            onValueChanged: controller.fov = value;
        }
    }

    function updateHorizonLock() {
        const lockAmount = horizonCb.checked? horizonSlider.value : 0.0;
        const roll = horizonCb.checked? horizonRollSlider.value : 0.0;
        controller.set_horizon_lock(lockAmount, roll);
    }

    ComboBox {
        id: smoothingMethod;
        model: smoothingAlgorithms;
        font.pixelSize: 12 * dpiScale;
        width: parent.width;
        currentIndex: 1;
        Component.onCompleted: currentIndexChanged();
        onCurrentIndexChanged: {
            // Clear current params
            for (let i = smoothingOptions.children.length; i > 0; --i) {
                smoothingOptions.children[i - 1].destroy();
            }

            const opt_json = controller.set_smoothing_method(currentIndex);
            if (opt_json.length > 0) {
                let qml = "import QtQuick; import '../components/'; Column { width: parent.width; ";
                let adv_qml = "AdvancedSection { diff: 0; ";
                for (const x of opt_json) {
                    // TODO: figure out a better way than constructing a string
                    let str = "";
                    const add = x.custom_qml || "";
                    switch (x.type) {
                        case 'Slider': 
                        case 'SliderWithField': 
                        case 'NumberField':
                            str = `Label {
                                width: parent.width;
                                spacing: 2 * dpiScale;
                                text: qsTranslate("Stabilization", "${x.description}");
                                objectName: "param-${x.name}-label";
                                ${x.type} {
                                    width: parent.width;
                                    from: ${x.from};
                                    to: ${x.to};
                                    value: root.getSmoothingParam("${x.name}", ${x.value});
                                    defaultValue: ${x.default};
                                    objectName: "param-${x.name}";
                                    unit: qsTranslate("Stabilization", "${x.unit}");
                                    precision: ${x.precision} || 2;
                                    onValueChanged: root.setSmoothingParam("${x.name}", value);
                                    ${add}
                                }
                            }`;
                        break;
                        case 'CheckBox':
                            str = `CheckBox {
                                text: qsTranslate("Stabilization", "${x.description}")
                                checked: +root.getSmoothingParam("${x.name}", ${x.default}) > 0;
                                onCheckedChanged: root.setSmoothingParam("${x.name}", checked? 1 : 0);
                                objectName: "param-${x.name}";
                                Component.onCompleted: checkedChanged();
                                ${add}
                            }`;
                        break;
                        case 'QML': str = x.custom_qml; break;
                    }
                    if (x.advanced) adv_qml += str
                    else qml += str;
                }
                qml += adv_qml.length > 40? (adv_qml + "}") : "";
                qml += "}";

                Qt.createQmlObject(qml, smoothingOptions);

                Qt.callLater(updateHorizonLock);
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
    
    Column {
        id: horizonLock;
        x: 5 * dpiScale;
        width: parent.width - x;
        visible: children.length > 0;
    }

    CheckBoxWithContent {
        id: horizonCb;
        text: qsTr("Lock horizon");

        cb.onCheckedChanged: {
            updateHorizonLock();
        }

        Label {
            text: qsTr("Lock amount", "Horizon locking amount");
            width: parent.width;
            spacing: 2 * dpiScale;
            SliderWithField {
                id: horizonSlider;
                defaultValue: 100;
                to: 100;
                width: parent.width;
                unit: qsTr("%");
                precision: 0;
                value: 100;
                onValueChanged: updateHorizonLock();
            }
        }

        Label {
            width: parent.width;
            spacing: 2 * dpiScale;
            text: qsTr("Roll angle correction")
            SliderWithField {
                id: horizonRollSlider;
                width: parent.width;
                from: -180;
                to: 180;
                value: 0;
                defaultValue: 0;
                unit: qsTr("°");
                precision: 1;
                onValueChanged: updateHorizonLock();
            }
        }

        BasicText {
            width: parent.width;
            wrapMode: Text.WordWrap;
            textFormat: Text.StyledText;
            text: qsTr("Requires accurate orientation determination. Try with Complementary, Mahony, or Madgwick integration method.");
        }
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
        model: [QT_TRANSLATE_NOOP("Popup", "No zooming"), QT_TRANSLATE_NOOP("Popup", "Dynamic zooming"), QT_TRANSLATE_NOOP("Popup", "Static zoom")];
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
        text: qsTr("Zooming speed");
        visible: croppingMode.currentIndex == 1;
        SliderWithField {
            id: adaptiveZoom;
            value: 4;
            defaultValue: 4;
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
                defaultValue: 0;
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
