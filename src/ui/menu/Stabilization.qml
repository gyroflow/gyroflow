import QtQuick 2.15
import Qt.labs.settings 1.0

import "../components/"

MenuItem {
    id: root;
    text: qsTr("Stabilization");
    icon: "gyroflow";
    innerItem.enabled: window.videoArea.vid.loaded;

    Settings {
        id: settings;
        property alias smoothingMethod: smoothingMethod.currentIndex;
        property alias croppingMode: croppingMode.currentIndex;
        property alias adaptiveZoom: adaptiveZoom.value;
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
        function onTelemetry_loaded(is_main_video, filename, camera, imu_orientation, contains_gyro, contains_quats, frame_readout_time) {
            setShutterTimer.pending = frame_readout_time;
            setShutterTimer.start();
        }
        function onRolling_shutter_estimated(rolling_shutter) {
            shutter.value = Math.abs(rolling_shutter);
            shutterCb.checked = Math.abs(rolling_shutter) > 0;
            bottomToTop.checked = rolling_shutter < 0;
        }
    }
    Timer {
        id: setShutterTimer;
        property real pending: 0;
        interval: 2000;
        repeat: false;
        running: false;
        onTriggered: {
            shutter.value = Math.abs(pending);
            shutterCb.checked = Math.abs(pending) > 0;
            bottomToTop.checked = pending < 0;
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
        Slider {
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
        currentIndex: 1;
        Component.onCompleted: currentIndexChanged();
        onCurrentIndexChanged: {
            // Clear current params
            for (let i = smoothingOptions.children.length; i > 0; --i) {
                smoothingOptions.children[i - 1].destroy();
            }

            const opt_json = controller.set_smoothing_method(currentIndex);
            if (opt_json.length > 0) {
                let qml = "import QtQuick 2.15; import '../components/'; Column { width: parent.width; ";
                for (const x of opt_json) {
                    // TODO: figure out a better way than constructing a string
                    switch (x.type) {
                        case 'Slider': 
                        case 'SliderWithField': 
                        case 'NumberField': 
                            qml += `Label {
                                width: parent.width;
                                spacing: 2 * dpiScale;
                                text: qsTr("${x.description}")
                                ${x.type} {
                                    width: parent.width;
                                    from: ${x.from};
                                    to: ${x.to};
                                    value: root.getSmoothingParam("${x.name}", ${x.value});
                                    defaultValue: ${x.value};
                                    unit: "${x.unit}";
                                    live: false;
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

    ComboBox {
        id: croppingMode;
        font.pixelSize: 12 * dpiScale;
        width: parent.width;
        model: [qsTr("No cropping"), qsTr("Dynamic cropping"), qsTr("Static crop")];
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
        Slider {
            id: adaptiveZoom;
            value: 4;
            from: 0.1;
            to: 15;
            unit: "s";
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
                unit: "ms";
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
