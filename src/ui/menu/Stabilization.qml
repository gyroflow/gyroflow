import QtQuick 2.15

import "../components/"

MenuItem {
    text: qsTr("Stabilization");
    icon: "gyroflow";
    enabled: window.videoArea.vid.loaded;

    Connections {
        target: controller;
        function onTelemetry_loaded(is_main_video, filename, camera, imu_orientation, contains_gyro, contains_quats, frame_readout_time) {
            setShutterTimer.pending = frame_readout_time;
            setShutterTimer.start();
        }
    }
    Timer {
        id: setShutterTimer;
        property real pending: 0;
        interval: 2000;
        repeat: false;
        running: false;
        onTriggered: {
            shutter.value = pending;
            shutterCb.checked = Math.abs(pending) > 0;
        }
    }

    WarningMessage {
        id: fovWarning;
        visible: opacity > 0;
        opacity: fov.value > 1.0 && croppingMode.currentIndex > 0? 1 : 0;
        Ease on opacity { }
        height: (t.height + 10 * dpiScale) * opacity - parent.spacing * (1.0 - opacity);
        t.font.pixelSize: 12 * dpiScale;
        t.x: 5 * dpiScale;
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
                    switch (x.type) {
                        case 'Slider': 
                        case 'NumberField': 
                            qml += `Label {
                                width: parent.width;
                                text: qsTr("${x.description}")
                                ${x.type} {
                                    width: parent.width;
                                    from: ${x.from};
                                    to: ${x.to};
                                    value: ${x.value};
                                    unit: "${x.unit}";
                                    live: false;
                                    ${x.type == "NumberField"? "precision: " + x.precision : ""}
                                    onValueChanged: controller.set_smoothing_param("${x.name}", value);
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
            Slider {
                id: shutter;
                to: 1000 / Math.max(1, window.videoArea.vid.frameRate);
                width: parent.width;
                unit: "ms";
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
