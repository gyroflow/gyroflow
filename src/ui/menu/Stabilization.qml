// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import Qt.labs.settings

import "../components/"

MenuItem {
    id: root;
    text: qsTr("Stabilization");
    iconName: "gyroflow";
    innerItem.enabled: window.videoArea.vid.loaded;
    objectName: "stabilization";

    property alias horizonCb: horizonCb;
    property alias horizonRollSlider: horizonRollSlider;
    property alias fovSlider: fov;
    property alias maxValues: maxValues;
    property alias videoSpeed: videoSpeed;
    property alias zoomingCenterX: zoomingCenterX;
    property alias zoomingCenterY: zoomingCenterY;
    property alias croppingMode: croppingMode;

    Settings {
        id: settings;
        property alias smoothingMethod: smoothingMethod.currentIndex;
        property alias croppingMode: croppingMode.currentIndex;
        property alias adaptiveZoom: adaptiveZoom.value;
        property alias correctionAmount: correctionAmount.value;
        property alias useGravityVectors: useGravityVectors.checked;
        property alias hlIntegrationMethod: integrationMethod.currentIndex;
        property alias videoSpeedAffectsSmoothing: videoSpeedAffectsSmoothing.checked;
        property alias videoSpeedAffectsZooming: videoSpeedAffectsZooming.checked;
        property alias zoomingMethod: zoomingMethod.currentIndex;
    }

    function loadGyroflow(obj) {
        const stab = obj.stabilization || { };
        if (stab && Object.keys(stab).length > 0) {
            if (stab.hasOwnProperty("fov")) fov.value = +stab.fov;
            const methodIndex = smoothingAlgorithms.indexOf(stab.method);
            if (methodIndex > -1) {
                smoothingMethod.currentIndex = methodIndex;
            }
            if (stab.smoothing_params) {
                Qt.callLater(function() {
                    for (const x of stab.smoothing_params) {
                        const el = root.getParamElement(x.name);
                        if (el) {
                            console.log("Setting param", x.name, x.value);
                            if (el.value) el.value = x.value;
                            if (el.checked) el.checked = +x.value > 0;
                        }
                    }
                });
            }
            if (typeof stab.frame_readout_time === 'number') {
                setFrameReadoutTime(+stab.frame_readout_time);
            }

            if (typeof stab.lens_correction_amount !== "undefined") {
                correctionAmount.value = +stab.lens_correction_amount;
            }

            const az = +stab.adaptive_zoom_window;
            if (az < -0.9) {
                croppingMode.currentIndex = 2; // Static crop
            } else if (az > 0) {
                croppingMode.currentIndex = 1; // Dynamic cropping
                adaptiveZoom.value = az;
            } else {
                croppingMode.currentIndex = 0; // No cropping
            }
            if (stab.hasOwnProperty("adaptive_zoom_center_offset")) {
                zoomingCenterX.value = stab.adaptive_zoom_center_offset[0];
                zoomingCenterY.value = stab.adaptive_zoom_center_offset[1];
            }
            if (stab.hasOwnProperty("adaptive_zoom_method")) zoomingMethod.currentIndex = +stab.adaptive_zoom_method;
            if (stab.hasOwnProperty("use_gravity_vectors")) {
                useGravityVectors.checked = !!stab.use_gravity_vectors;
            }
            if (stab.hasOwnProperty("horizon_lock_integration_method")) {
                integrationMethod.currentIndex = stab.horizon_lock_integration_method;
            }

            horizonCb.checked = (+stab.horizon_lock_amount || 0) > 0;
            horizonSlider.value = horizonCb.checked? +stab.horizon_lock_amount : 100;
            horizonRollSlider.value = horizonCb.checked? +stab.horizon_lock_roll : 0;
            Qt.callLater(updateHorizonLock);

            if (stab.hasOwnProperty("video_speed")) videoSpeed.value = +stab.video_speed;
            if (stab.hasOwnProperty("video_speed_affects_smoothing")) videoSpeedAffectsSmoothing.checked = !!stab.video_speed_affects_smoothing;
            if (stab.hasOwnProperty("video_speed_affects_zooming"))   videoSpeedAffectsZooming.checked   = !!stab.video_speed_affects_zooming;

        }
    }

    function setFrameReadoutTime(v: real) {
        shutter.value = Math.abs(v);
        shutterCb.checked = Math.abs(v) > 0;
        bottomToTop.checked = v < 0;
    }

    function setSmoothingParam(name: string, value: real) {
        settings.setValue("smoothing-" + smoothingMethod.currentIndex + "-" + name, value);
        controller.set_smoothing_param(name, value);
    }
    function getSmoothingParam(name: string, defaultValue: real): real {
        return settings.value("smoothing-" + smoothingMethod.currentIndex + "-" + name, defaultValue);
    }
    function traverseChildren(node: QtObject, name: string): QtObject {
        for (let i = node.children.length; i > 0; --i) {
            const child = node.children[i - 1];
            if (child) {
                if (child.objectName == name) {
                    return child;
                }
                const found = traverseChildren(child, name);
                if (found !== null) return found;
            }
        }
        return null;
    }
    function getParamElement(name: string): QtObject {
        return traverseChildren(smoothingOptions, "param-" + name);
    }

    Connections {
        target: controller;
        function onCompute_progress(id: real, progress: real) {
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
        function onTelemetry_loaded(is_main_video: bool, filename: string, camera: string, additional_data: var) {
            if (is_main_video) {
                if (Math.abs(+additional_data.frame_readout_time) > 0) {
                    root.setFrameReadoutTime(additional_data.frame_readout_time);
                } else {
                    controller.frame_readout_time = shutter.value;
                }
            }
        }
        function onRolling_shutter_estimated(rolling_shutter: real) {
            root.setFrameReadoutTime(rolling_shutter);
        }
    }

    Component.onCompleted: {
        QT_TRANSLATE_NOOP("Popup", "No smoothing");
        QT_TRANSLATE_NOOP("Popup", "Default"),
        QT_TRANSLATE_NOOP("Popup", "Plain 3D");
        QT_TRANSLATE_NOOP("Popup", "Fixed camera");

        QT_TRANSLATE_NOOP("Stabilization", "Pitch smoothness");
        QT_TRANSLATE_NOOP("Stabilization", "Yaw smoothness");
        QT_TRANSLATE_NOOP("Stabilization", "Roll smoothness");
        QT_TRANSLATE_NOOP("Stabilization", "Smoothness");
        QT_TRANSLATE_NOOP("Stabilization", "Per axis");
        QT_TRANSLATE_NOOP("Stabilization", "Max smoothness");
        QT_TRANSLATE_NOOP("Stabilization", "Max smoothness at high velocity");
        QT_TRANSLATE_NOOP("Stabilization", "Second smoothing pass");
        QT_TRANSLATE_NOOP("Stabilization", "Only within trim range");
        QT_TRANSLATE_NOOP("Stabilization", "Yaw angle correction");
        QT_TRANSLATE_NOOP("Stabilization", "Pitch angle correction");
        QT_TRANSLATE_NOOP("Stabilization", "Roll angle correction");
        QT_TRANSLATE_NOOP("Stabilization", "Yaw angle");
        QT_TRANSLATE_NOOP("Stabilization", "Pitch angle");
        QT_TRANSLATE_NOOP("Stabilization", "Roll angle");
    }

    InfoMessageSmall {
        id: fovWarning;
        show: fov.value > 1.0 && croppingMode.currentIndex > 0;
        text: qsTr("FOV is greater than 1.0, you may see black borders");
    }

    Label {
        position: Label.LeftPosition;
        text: qsTr("FOV");
        SliderWithField {
            id: fov;
            from: 0.1;
            to: 3;
            value: 1.0;
            defaultValue: 1.0;
            width: parent.width;
            keyframe: "Fov";
            onValueChanged: controller.fov = value;
        }
    }

    function updateHorizonLock() {
        const lockAmount = horizonCb.checked? horizonSlider.value : 0.0;
        const roll = horizonCb.checked? horizonRollSlider.value : 0.0;
        controller.set_horizon_lock(lockAmount, roll);
        controller.set_use_gravity_vectors(useGravityVectors.checked);
        controller.set_horizon_lock_integration_method(integrationMethod.currentIndex);
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
                            const kf = (x.type == 'SliderWithField' || x.type == 'NumberField') && x.keyframe? `keyframe: "${x.keyframe}";` : "";
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
                                    ${kf}
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

        cb.onCheckedChanged: Qt.callLater(updateHorizonLock);

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
                keyframe: "LockHorizonAmount";
                onValueChanged: Qt.callLater(updateHorizonLock);
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
                keyframe: "LockHorizonRoll";
                onValueChanged: Qt.callLater(updateHorizonLock);
            }
        }
        CheckBox {
            id: useGravityVectors;
            text: qsTr("Use gravity vectors");
            checked: false;
            visible: controller.has_gravity_vectors;
            onCheckedChanged: Qt.callLater(updateHorizonLock);
        }

        Label {
            position: Label.LeftPosition;
            text: qsTr("Integration method");
            property bool usesQuats: window.motionData.hasQuaternions && window.motionData.hasRawGyro && window.motionData.integrationMethod === 0;
            visible: usesQuats;

            ComboBox {
                id: integrationMethod;
                model: ["Complementary", "VQF", "Simple gyro + accel", "Mahony", "Madgwick"];
                currentIndex: 1;
                font.pixelSize: 12 * dpiScale;
                width: parent.width;
                tooltip: qsTr("IMU integration method for keeping track of the horizon and adjust built-in quaternions");
                onCurrentIndexChanged: Qt.callLater(updateHorizonLock);
            }
        }

        BasicText {
            width: parent.width;
            wrapMode: Text.WordWrap;
            textFormat: Text.StyledText;
            text: qsTr("If the horizon is not locked well, try a different integration method in the \"Motion data\" section.");
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
        currentIndex: 1;
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
            if (currentIndex == 0) {
                zoomingCenterX.value = 0;
                zoomingCenterY.value = 0;
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
            keyframe: "ZoomingSpeed";
            onValueChanged: controller.adaptive_zoom = value;
            onKeyframesEnabledChanged: Qt.callLater(zoomingMethod.adjustMethod);
        }
    }

    AdvancedSection {
        InfoMessageSmall {
            show: croppingMode.currentIndex == 1 && zoomingMethod.currentIndex == 0 && zoomingMethod.zoomingSpeedKeyframed;
            text: qsTr("When keyframing zooming speed, it is recommended to use the Envelope follower method. Gaussian filter might lead to black borders in view.");
        }

        Label {
            position: Label.LeftPosition;
            text: qsTr("Zooming method");
            visible: croppingMode.currentIndex == 1;
            ComboBox {
                id: zoomingMethod;
                model: ["Gaussian filter", "Envelope follower"];
                // font.pixelSize: 12 * dpiScale;
                width: parent.width;
                currentIndex: 0;
                onCurrentIndexChanged: controller.zooming_method = currentIndex;
                property bool zoomingSpeedKeyframed: adaptiveZoom.keyframesEnabled || (videoSpeed.keyframesEnabled && videoSpeedAffectsSmoothing.checked);
                function adjustMethod() {
                    // If keyframes are enabled, change to Envelope follower by default
                    if (zoomingSpeedKeyframed && zoomingMethod.currentIndex == 0) {
                        currentIndex = 1;
                    }
                }
            }
        }

        Label {
            text: qsTr("Zooming center offset");
            Column {
                width: parent.width;
                Label {
                    text: qsTr("X");
                    position: Label.LeftPosition;
                    SliderWithField {
                        id: zoomingCenterX;
                        precision: 2;
                        value: 0;
                        defaultValue: 0;
                        from: -100;
                        to: 100;
                        unit: qsTr("%");
                        width: parent.width;
                        keyframe: "ZoomingCenterX";
                        scaler: 100.0;
                        onValueChanged: controller.zooming_center_x = value;
                    }
                }
                Label {
                    text: qsTr("Y");
                    position: Label.LeftPosition;
                    SliderWithField {
                        id: zoomingCenterY;
                        precision: 2;
                        value: 0;
                        defaultValue: 0;
                        from: -100;
                        to: 100;
                        unit: qsTr("%");
                        width: parent.width;
                        keyframe: "ZoomingCenterY";
                        scaler: 100.0;
                        onValueChanged: controller.zooming_center_y = value;
                    }
                }
            }
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
                to: 1000 / Math.max(1, window.videoArea.timeline.scaledFps);
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

    Label {
        text: qsTr("Lens correction strength");
        SliderWithField {
            id: correctionAmount;
            from: 0.0;
            to: 100.0;
            value: 1.0;
            unit: "%";
            defaultValue: 100.0;
            precision: 0;
            width: parent.width;
            keyframe: "LensCorrectionStrength";
            scaler: 100.0;
            onValueChanged: Qt.callLater(() => { controller.lens_correction_amount = value; });
        }
    }

    Label {
        text: qsTr("Video speed");
        SliderWithField {
            id: videoSpeed;
            from: 10;
            to: 1000.0;
            value: 1.0;
            unit: "%";
            defaultValue: 100.0;
            precision: 0;
            width: parent.width;
            keyframe: "VideoSpeed";
            scaler: 100.0;
            property bool isKeyframed: false;
            function updateVideoSpeed() {
                window.videoArea.vid.playbackRate = videoSpeed.value;
                controller.set_video_speed(videoSpeed.value, videoSpeedAffectsSmoothing.checked, videoSpeedAffectsZooming.checked);
                isKeyframed = controller.is_keyframed("VideoSpeed");
            }
            Timer {
                id: speedUpdateTimer;
                interval: 300;
                onTriggered: Qt.callLater(videoSpeed.updateVideoSpeed);
            }
            slider.onPressedChanged: if (!slider.pressed) Qt.callLater(videoSpeed.updateVideoSpeed);
            onValueChanged: speedUpdateTimer.restart();
            onKeyframesEnabledChanged: Qt.callLater(zoomingMethod.adjustMethod);
            Connections {
                target: controller;
                function onKeyframe_value_updated(keyframe: string, value: real) {
                    if (keyframe == "VideoSpeed") {
                        if (Math.abs(window.videoArea.vid.playbackRate - value) > 0.005) {
                            window.videoArea.vid.playbackRate = value;
                        }
                    }
                }
            }
        }
        CheckBox {
            id: videoSpeedAffectsSmoothing;
            anchors.right: parent.right;
            anchors.top: parent.top;
            anchors.topMargin: -30 * dpiScale;
            anchors.rightMargin: -15 * dpiScale;
            contentItem.visible: false;
            scale: 0.7;
            tooltip: qsTr("Link with smoothing");
            checked: true;
            onCheckedChanged: Qt.callLater(videoSpeed.updateVideoSpeed);
        }
        CheckBox {
            id: videoSpeedAffectsZooming;
            anchors.right: parent.right;
            anchors.top: parent.top;
            anchors.topMargin: -30 * dpiScale;
            anchors.rightMargin: 15 * dpiScale;
            width: 25 * dpiScale;
            contentItem.visible: false;
            scale: 0.7;
            tooltip: qsTr("Link with zooming speed");
            checked: true;
            onCheckedChanged: Qt.callLater(videoSpeed.updateVideoSpeed);
        }
    }
}
