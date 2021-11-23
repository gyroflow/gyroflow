import QtQuick 2.15

import "../components/"

MenuItem {
    text: qsTr("Stabilization");
    icon: "gyroflow";
    enabled: window.videoArea.vid.loaded;

    Connections {
        target: controller;
        function onTelemetry_loaded(is_main_video, filename, camera, imu_orientation, contains_gyro, contains_quats, frame_readout_time) {
            shutter.value = frame_readout_time;
            shutterCb.checked = Math.abs(frame_readout_time) > 0;
        }
    }

    Label {
        position: Label.Left;
        text: qsTr("FOV");
        Slider {
            from: 0.1;
            to: 3;
            value: 1.0;
            width: parent.width;
            onValueChanged: controller.fov = value;
        }
    }

    ComboBox {
        model: smoothingAlgorithms;
        font.pixelSize: 12 * dpiScale;
        width: parent.width;
        Component.onCompleted: currentIndex = 1;
        onCurrentIndexChanged: {
            // Clear current params
            for (let i = smoothingOptions.children.length; i > 0; --i) {
                smoothingOptions.children[i - 1].destroy();
            }

            const opt_json = controller.set_smoothing_method(currentIndex);
            if (opt_json.length > 0) {
                let qml = "import QtQuick 2.15; import '../components/'; Column { width: parent.width; ";
                for (const x of opt_json) {
                    // TODO figure out a better way than constructing a string
                    qml += `Label {
                        width: parent.width;
                        text: qsTr("${x.description}")
                        Slider {
                            width: parent.width;
                            from: ${x.from};
                            to: ${x.to};
                            value: ${x.value};
                            unit: "${x.unit}";
                            onValueChanged: controller.set_smoothing_param("${x.name}", value);
                        }
                    }`;
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

    CheckBoxWithContent {
        id: shutterCb;
        text: qsTr("Rolling shutter correction");
        cb.onCheckedChanged: {
            controller.frame_readout_time = cb.checked? shutter.value : 0.0;
        }

        Label {
            text: qsTr("Frame readout time");
            Slider {
                id: shutter;
                from: -to;
                to: 1000 / Math.max(1, window.videoArea.vid.frameRate);
                width: parent.width;
                unit: "ms";
                onValueChanged: controller.frame_readout_time = value;
            }
        }
    }
    
    ComboBox {
        id: croppingMode;
        font.pixelSize: 12 * dpiScale;
        width: parent.width;
        model: [qsTr("No cropping"), qsTr("Dynamic cropping"), qsTr("Static crop")];
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
}
