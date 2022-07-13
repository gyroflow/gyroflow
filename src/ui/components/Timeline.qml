// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Controls as QQC
import QtQuick.Controls.Material as QQCM
import Qt.labs.settings

import Gyroflow

// TODO: multiple trims

Item {
    id: root;
    property real trimStart: 0.0;
    property real trimEnd: 1.0;
    property bool trimActive: trimStart > 0.01 || trimEnd < 0.99;

    property real durationMs: 0;
    property real orgDurationMs: 0;

    property real visibleAreaLeft: 0.0;
    property real visibleAreaRight: 1.0;
    onVisibleAreaLeftChanged: Qt.callLater(redrawChart);
    onVisibleAreaRightChanged: Qt.callLater(redrawChart);
    property alias pressed: ma.pressed;
    property alias inner: inner;

    property bool fullScreen: false;

    property real value: 0;
    readonly property real position: vid.timestamp / root.orgDurationMs;

    function mapToVisibleArea(pos: real): real { return (pos - visibleAreaLeft) / (visibleAreaRight - visibleAreaLeft); }
    function mapFromVisibleArea(pos: real): real { return pos * (visibleAreaRight - visibleAreaLeft) + visibleAreaLeft; }

    function redrawChart() { chart.update(); keyframes.update(); }
    function getChart(): TimelineGyroChart { return chart; }
    function getKeyframesView(): TimelineKeyframesView { return keyframes; }

    function getTimestampUs(): real {
        return vid.timestamp * 1000;
    }
    function setPosition(pos: real) {
        vid.currentFrame = frameAtPosition(pos);
    }
    function frameAtPosition(pos: real): int {
        return Math.floor(pos * (vid.frameCount - 1));
    }

    function timeAtPosition(pos: real): string {
        const time = Math.max(0, durationMs * pos);
        return new Date(time).toISOString().substring(11, 11+8);
    }

    function setTrim(start: real, end: real) {
        if (start >= end) {
            resetTrim();
        } else {
            trimStart = start;
            trimEnd   = end;
        }
    }

    function resetTrim() {
        root.trimStart = 0;
        root.trimEnd = 1.0;
    }

    function toggleAxis(axis: int, solo: bool) {
        let v = (chart.getAxisVisible(axis) ? 1 : 0) + (chart.getAxisVisible(axis + 4) ? 2 : 0);
        v = (v + 1) % 4;
        chart.setAxisVisible(axis, v & 1);
        chart.setAxisVisible(axis + 4, v & 2);
        if (solo) {
            for (let i = 0; i < 8; i++) {
                if (i % 4 != axis)
                    chart.setAxisVisible(i, false);
            }
        }
    }

    function updateDurations() {
        chart.setDurationMs(controller.get_scaled_duration_ms());
        keyframes.setDurationMs(controller.get_org_duration_ms());
        root.durationMs    = controller.get_scaled_duration_ms();
        root.orgDurationMs = controller.get_org_duration_ms();

        Qt.callLater(controller.update_chart, chart);
        Qt.callLater(controller.update_keyframes_view, keyframes);
    }

    Settings {
        property alias timelineChart: chart.viewMode;
    }

    focus: true;

    Column {
        visible: !root.fullScreen;
        x: 3 * dpiScale;
        y: 50 * dpiScale;
        spacing: 3 * dpiScale;
        TimelineAxisButton { id: a0; text: "X"; onCheckedChanged: chart.setAxisVisible(0, checked); checked: chart.getAxisVisible(0); }
        TimelineAxisButton { id: a1; text: "Y"; onCheckedChanged: chart.setAxisVisible(1, checked); checked: chart.getAxisVisible(1); }
        TimelineAxisButton { id: a2; text: "Z"; onCheckedChanged: chart.setAxisVisible(2, checked); checked: chart.getAxisVisible(2); }
        TimelineAxisButton { id: a3; text: "W"; onCheckedChanged: chart.setAxisVisible(3, checked); checked: chart.getAxisVisible(3); }
    }
    Column {
        visible: !root.fullScreen;
        anchors.right: parent.right;
        anchors.rightMargin: 3 * dpiScale;
        y: 50 * dpiScale;
        spacing: 3 * dpiScale;
        TimelineAxisButton { id: a4; text: "X"; onCheckedChanged: chart.setAxisVisible(4, checked); checked: chart.getAxisVisible(4); }
        TimelineAxisButton { id: a5; text: "Y"; onCheckedChanged: chart.setAxisVisible(5, checked); checked: chart.getAxisVisible(5); }
        TimelineAxisButton { id: a6; text: "Z"; onCheckedChanged: chart.setAxisVisible(6, checked); checked: chart.getAxisVisible(6); }
        TimelineAxisButton { id: a7; text: "W"; onCheckedChanged: chart.setAxisVisible(7, checked); checked: chart.getAxisVisible(7); }
    }

    Item {
        id: inner;
        x: (root.fullScreen? 10 : 33) * dpiScale;
        y: 15 * dpiScale;
        width: parent.width - x - (root.fullScreen? 10 : 33) * dpiScale;
        height: parent.height - y - 30 * dpiScale - parent.additionalHeight;

        Rectangle {
            x: 0;
            y: (root.fullScreen? 0 : 35) * dpiScale;
            width: parent.width
            radius: 4 * dpiScale;
            color: root.fullScreen? "transparent" : Qt.lighter(styleButtonColor, 1.1)
            height: parent.height - y;
            opacity: root.trimActive? 0.9 : 1.0;

            TimelineGyroChart {
                id: chart;
                visibleAreaLeft: root.visibleAreaLeft;
                visibleAreaRight: root.visibleAreaRight;
                anchors.fill: parent;
                anchors.topMargin: (root.fullScreen? 0 : 5) * dpiScale;
                anchors.bottomMargin: (root.fullScreen? 0 : 5) * dpiScale;
                opacity: root.trimActive? 0.9 : 1.0;
                onAxisVisibleChanged: {
                    a0.checked = chart.getAxisVisible(0);
                    a1.checked = chart.getAxisVisible(1);
                    a2.checked = chart.getAxisVisible(2);
                    a3.checked = chart.getAxisVisible(3);
                    a4.checked = chart.getAxisVisible(4);
                    a5.checked = chart.getAxisVisible(5);
                    a6.checked = chart.getAxisVisible(6);
                    a7.checked = chart.getAxisVisible(7);
                }
            }
            TimelineKeyframesView {
                id: keyframes;
                videoTimestamp: vid.timestamp;
                visibleAreaLeft: root.visibleAreaLeft;
                visibleAreaRight: root.visibleAreaRight;
                anchors.fill: parent;
                anchors.topMargin: (root.fullScreen? 0 : 5) * dpiScale;
                anchors.bottomMargin: (root.fullScreen? 0 : 5) * dpiScale;
                function handleMouseMove(x: real, y: real, pressed: bool, pressedButtons: int): bool {
                    const pt = ma.mapToItem(keyframes, x, y);
                    const kf = keyframes.keyframeAtXY(pt.x, pt.y);
                    if (kf) {
                        const [keyframe, timestamp, name, value] = kf.split(":", 4);
                        if (pressed && (pressedButtons & Qt.RightButton)) {
                            keyframeContextMenu.pressedKeyframe = keyframe;
                            keyframeContextMenu.pressedKeyframeTs = timestamp;
                            keyframeContextMenu.updateEasingMenu();
                            keyframeContextMenu.popup();
                            return true;
                        }
                        if (pressed && (pressedButtons & Qt.LeftButton)) {
                            vid.setTimestamp(timestamp / 1000);
                            return true;
                        }
                        ma.cursorShape = Qt.PointingHandCursor;
                        if (!kftt.visible) {
                            kftt.x       = pt.x + 10 * dpiScale;
                            kftt.offsetY = pt.y + 10 * dpiScale + kftt.height;
                            kftt.text = qsTr(name) + " - " + value;
                            kftt.visible = true;
                        }
                    } else {
                        ma.cursorShape = Qt.ArrowCursor;
                        if (kftt.visible)
                            kftt.visible = false;
                    }
                    return false;
                }
                ToolTip { id: kftt; z: 5; }
                Menu {
                    id: keyframeContextMenu;
                    property string pressedKeyframe: "";
                    property real pressedKeyframeTs: 0;
                    z: 6;

                    font.pixelSize: 11.5 * dpiScale;
                    Action {
                        icon.name: "bin;#f67575";
                        text: qsTr("Delete");
                        onTriggered: controller.remove_keyframe(keyframeContextMenu.pressedKeyframe, keyframeContextMenu.pressedKeyframeTs);
                    }
                    Action {
                        id: easeIn;
                        icon.name: "ease_in";
                        text: qsTr("Ease in");
                        checkable: true;
                        onTriggered: keyframeContextMenu.updateEasing();
                    }
                    Action {
                        id: easeOut;
                        icon.name: "ease_out";
                        text: qsTr("Ease out");
                        checkable: true;
                        onTriggered: keyframeContextMenu.updateEasing();
                    }
                    function updateEasingMenu() {
                        let e = controller.keyframe_easing(pressedKeyframe, pressedKeyframeTs);
                        easeIn.checked  = e == "EaseIn"  || e == "EaseInOut";
                        easeOut.checked = e == "EaseOut" || e == "EaseInOut";
                    }
                    function updateEasing() {
                        let e = "NoEasing";
                        if (easeIn.checked) e = "EaseIn";
                        if (easeOut.checked) e = "EaseOut";
                        if (easeIn.checked && easeOut.checked) e = "EaseInOut";
                        controller.set_keyframe_easing(pressedKeyframe, pressedKeyframeTs, e);
                    }
                }
                Component.onCompleted: {
                    QT_TR_NOOP("FOV");
                    QT_TR_NOOP("Video rotation");
                    QT_TR_NOOP("Zooming speed");
                    QT_TR_NOOP("Zooming center offset X");
                    QT_TR_NOOP("Zooming center offset Y");
                    QT_TR_NOOP("Background margin");
                    QT_TR_NOOP("Background feather");
                    QT_TR_NOOP("Horizon lock amount");
                    QT_TR_NOOP("Horizon lock roll correction");
                    QT_TR_NOOP("Lens correction strength");
                    QT_TR_NOOP("Max smoothness");
                    QT_TR_NOOP("Max smoothness at high velocity");
                    QT_TR_NOOP("Smoothness");
                    QT_TR_NOOP("Smoothness pitch");
                    QT_TR_NOOP("Smoothness roll");
                    QT_TR_NOOP("Smoothness yaw");
                }
            }
        }

        // Lines
        // TODO QQuickPaintedItem
        Column {
            width: parent.width;
            visible: !root.fullScreen;
            Row {
                width: parent.width;
                spacing: (100 * dpiScale) - children[0].width;
                x: -children[0].width / 2;
                //layer.enabled: true;
                Repeater {
                    model: Math.max(0, linesCanvas.bigLines + 1);
                    BasicText {
                        leftPadding: 0;
                        font.pixelSize: 10 * dpiScale;
                        opacity: 0.6;
                        text: timeAtPosition(root.mapFromVisibleArea(x / parent.width));
                    }
                }
            }

            Item {
                width: parent.width;
                height: 15 * dpiScale;
                Canvas {
                    id: linesCanvas;
                    width: parent.width*2;
                    height: parent.height*2;
                    scale: 0.5;
                    anchors.centerIn: parent;
                    transformOrigin: Item.Center;
                    contextType: "2d";
                    layer.enabled: true;
                    property int lines: width / (20 * dpiScale);
                    property int bigLines: lines / 10;

                    onPaint: {
                        let ctx = context;
                        if (ctx) {
                            ctx.reset();
                            for (let j = 0; j < lines; j++) {
                                const x = Math.round(j * 20 * dpiScale);
                                ctx.beginPath();
                                ctx.moveTo(x, (j % 10 == 0)? 0 : height / 2);
                                ctx.lineTo(x, height);
                                ctx.strokeStyle = "#444444";
                                ctx.lineWidth = 1;
                                ctx.closePath();
                                ctx.lineCap = "round";
                                ctx.stroke();
                            }
                        }
                    }
                }
            }
        }

        MouseArea {
            id: ma;
            anchors.fill: parent;
            hoverEnabled: true;
            acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton;
            anchors.topMargin: (root.fullScreen? -8 : 0) * dpiScale;
            anchors.bottomMargin: (root.fullScreen? -10 : 0) * dpiScale;

            property var panInit: ({ x: 0.0, y: 0.0, visibleAreaLeft: 0.0, visibleAreaWidth: 1.0 });

            onMouseXChanged: {
                if (pressed) {
                    if (cursorShape == Qt.PointingHandCursor) return; // Don't seek when clicking over a keyframe
                    if (pressedButtons & Qt.MiddleButton) {
                        const dx = mouseX - panInit.x;
                        const stepsPerPixel = panInit.visibleAreaWidth / parent.width;

                        visibleAreaLeft  = Math.max(0.0, Math.min(1.0 - panInit.visibleAreaWidth, panInit.visibleAreaLeft - dx * stepsPerPixel));
                        visibleAreaRight = visibleAreaLeft + panInit.visibleAreaWidth;

                        scrollbar.position = visibleAreaLeft;
                    } else {
                        const newPos = Math.max(0.0, Math.min(1.0, root.mapFromVisibleArea(mouseX / parent.width)));
                        const currentX = root.mapToVisibleArea(root.position) * parent.width;
                        if (pressedButtons & Qt.RightButton) {
                            if (Math.abs(mouseX - currentX) > 100) // If right click was more than 100px away from the current playhead
                                root.setPosition(newPos);
                        } else {
                            root.setPosition(newPos);
                        }
                    }
                } else {
                    Qt.callLater(keyframes.handleMouseMove, mouseX, mouseY, false, 0);
                }
            }
            onMouseYChanged: if (!pressed) Qt.callLater(keyframes.handleMouseMove, mouseX, mouseY, false, 0);
            onPressed: (mouse) => {
                panInit.x = mouse.x;
                panInit.y = mouse.y;
                panInit.visibleAreaLeft  = root.visibleAreaLeft;
                panInit.visibleAreaWidth = root.visibleAreaRight - root.visibleAreaLeft;
            }
            onPressAndHold: (mouse) => {
                if ((Qt.platform.os == "android" || Qt.platform.os == "ios") && mouse.button !== Qt.RightButton) {
                    timelineContextMenu.pressedX = mouse.x;
                    timelineContextMenu.popup()
                } else {
                    mouse.accepted = false;
                }
            }
            onClicked: (mouse) => {
                if (keyframes.handleMouseMove(mouse.x, mouse.y, true, mouse.button))
                    return;
                if (mouse.button === Qt.RightButton) {
                    timelineContextMenu.pressedX = mouse.x;
                    timelineContextMenu.popup();
                }
                root.focus = true;
            }
            onDoubleClicked: (mouse) => {
                root.visibleAreaLeft  = 0.0;
                root.visibleAreaRight = 1.0;
                chart.vscale = 1.0;
            }
            onWheel: (wheel) => {
                if ((wheel.modifiers & Qt.AltModifier) || (wheel.modifiers & Qt.MetaModifier)) {
                    const factor = (wheel.angleDelta.x / 120) / 10;
                    chart.vscale += factor;
                } else if ((wheel.modifiers & Qt.ControlModifier)) { // move horizontally
                    const remainingWindow = (root.visibleAreaRight - root.visibleAreaLeft);
                    const factor = (wheel.angleDelta.y / 120) / (50 / remainingWindow);
                    root.visibleAreaLeft  = Math.min(root.visibleAreaRight, Math.max(0.0, Math.min(1-remainingWindow, root.visibleAreaLeft - factor)));
                    root.visibleAreaRight = Math.max(root.visibleAreaLeft,  Math.min(1.0, Math.max(remainingWindow, root.visibleAreaRight - factor)));

                    scrollbar.position = root.visibleAreaLeft;
                } else { // zoom by default
                    const remainingWindow = (root.visibleAreaRight - root.visibleAreaLeft);

                    const factor = (wheel.angleDelta.y / 120) / (10 / remainingWindow);
                    const xPosFactor = wheel.x / root.width;
                    root.visibleAreaLeft  = Math.min(root.visibleAreaRight, Math.max(0.0, root.visibleAreaLeft  + factor * xPosFactor));
                    root.visibleAreaRight = Math.max(root.visibleAreaLeft,  Math.min(1.0, root.visibleAreaRight - factor * (1.0 - xPosFactor)));

                    scrollbar.position = root.visibleAreaLeft;
                }
            }
        }

        Menu {
            id: timelineContextMenu;
            property real pressedX: x;

            font.pixelSize: 11.5 * dpiScale;
            function setDisplayMode(i) {
                chart.viewMode = i;
                controller.update_chart(chart);
            }
            Action {
                id: addCalibAction;
                icon.name: "plus";
                text: qsTr("Add calibration point");
                onTriggered: {
                    const pos = root.position; // (root.mapFromVisibleArea(timelineContextMenu.pressedX / ma.width));
                    controller.add_calibration_point(pos * root.durationMs * 1000, calibrator_window.lensCalib.noMarker);
                }
            }
            QQC.MenuSeparator { id: msep; verticalPadding: 5 * dpiScale; }
            Action {
                id: syncHereAction;
                icon.name: "spinner";
                text: qsTr("Auto sync here");
                onTriggered: {
                    const pos = root.position; // (root.mapFromVisibleArea(timelineContextMenu.pressedX / ma.width));
                    controller.start_autosync(pos.toString(), window.sync.getSettingsJson(), "synchronize", window.exportSettings.overrideFps);
                }
            }
            Action {
                id: addSyncAction;
                icon.name: "plus";
                text: qsTr("Add manual sync point here");
                onTriggered: {
                    const pos = root.position * root.durationMs * 1000; // (root.mapFromVisibleArea(timelineContextMenu.pressedX / ma.width)) * root.durationMs * 1000;
                    const offset = controller.offset_at_video_timestamp(pos);
                    const final_pos = Math.round(pos - offset * 1000);
                    const final_offset = controller.offset_at_video_timestamp(final_pos)
                    controller.set_offset(final_pos, final_offset);
                    Qt.callLater(() => {
                        root.editingSyncPoint = true;
                        syncPointSlider.timestamp_us = final_pos;
                        syncPointSlider.from  = final_offset - Math.max(15, Math.abs(final_offset));
                        syncPointSlider.to    = final_offset + Math.max(15, Math.abs(final_offset));
                        syncPointSlider.value = final_offset;
                    });
                }
            }
            Action {
                id: guessOrientationHere;
                icon.name: "axes";
                text: qsTr("Guess IMU orientation here");
                onTriggered: {
                    const pos = root.position; // (root.mapFromVisibleArea(timelineContextMenu.pressedX / ma.width));
                    controller.start_autosync(pos.toString(), window.sync.getSettingsJson(), "guess_imu_orientation", window.exportSettings.overrideFps);
                }
            }
            Action {
                id: estimateRSAction;
                icon.name: "readout_time";
                text: qsTr("Estimate rolling shutter here");
                onTriggered: {
                    const pos = root.position; // (root.mapFromVisibleArea(timelineContextMenu.pressedX / ma.width));

                    const text = qsTr("Your video needs to be already synced properly and you should use this function\non a part of your video with significant camera motion (ideally horizontal).\n\n" +
                                      "This feature is experimental, the results may not be correct at all.\n" +
                                      "Are you sure you want to continue?");
                    messageBox(Modal.Warning, text, [
                        { text: qsTr("Yes"), clicked: function() {
                            controller.start_autosync(pos.toString(), window.sync.getSettingsJson(), "estimate_rolling_shutter", window.exportSettings.overrideFps);
                        }},
                        { text: qsTr("No"), accent: true },
                    ]);
                }
            }
            Action {
                id: debiasAction;
                icon.name: "bias";
                text: qsTr("Estimate gyro bias here");
                onTriggered: controller.estimate_bias(root.position);
            }
            Action {
                icon.name: "bin;#f67575";
                text: qsTr("Delete all sync points");
                onTriggered: controller.clear_offsets();
            }
            QQC.MenuSeparator { verticalPadding: 5 * dpiScale; }
            Menu {
                font.pixelSize: 11.5 * dpiScale;
                title: qsTr("Chart display mode")
                Action { checkable: true; checked: chart.viewMode === 0; text: qsTr("Gyroscope");     onTriggered: timelineContextMenu.setDisplayMode(0); }
                Action { checkable: true; checked: chart.viewMode === 1; text: qsTr("Accelerometer"); onTriggered: timelineContextMenu.setDisplayMode(1); }
                Action { checkable: true; checked: chart.viewMode === 2; text: qsTr("Magnetometer");  onTriggered: timelineContextMenu.setDisplayMode(2); }
                Action { checkable: true; checked: chart.viewMode === 3; text: qsTr("Quaternions");   onTriggered: timelineContextMenu.setDisplayMode(3); }
            }
            Component.onCompleted: {
                if (!isCalibrator) {
                    timelineContextMenu.removeAction(addCalibAction);
                    timelineContextMenu.removeItem(msep);
                }
            }
        }

        Item {
            anchors.fill: parent;
            clip: true;
            TimelineRangeIndicator {
                trimStart: root.trimStart;
                trimEnd: root.trimEnd;
                y: (root.fullScreen? 0 : 35) * dpiScale;
                height: parent.height - y;

                onActiveChanged: if (active) vid.setPlaybackRange(0, vid.duration);
                onTrimStartAdjustmentChanged: {
                    const dragPos = Math.max(0, trimStart + trimStartAdjustment);
                    if (mapToVisibleArea(dragPos) < 0 && dragPos >= 0) {
                        scrollbar.position = root.visibleAreaLeft = dragPos;
                    }
                    if (!vid.playing) root.setPosition(dragPos);
                }
                onTrimEndAdjustmentChanged: {
                    const dragPos = Math.min(1, trimEnd + trimEndAdjustment);
                    if (mapToVisibleArea(dragPos) > 1 && dragPos <= 1) {
                        root.visibleAreaRight = dragPos;
                    }
                    if (!vid.playing) root.setPosition(dragPos);
                }
                visible: root.trimActive;
                onChangeTrimStart: (val) => root.setTrim(val, root.trimEnd);
                onChangeTrimEnd: (val) => root.setTrim(root.trimStart, val);
                onReset: root.resetTrim();
            }
        }

        // Handle
        Rectangle {
            x: Math.max(0, (root.mapToVisibleArea(root.position) * parent.width) - width / 2)
            y: (parent.height - height) / 2
            radius: width;
            height: parent.height;
            width: 2 * dpiScale;
            color: styleAccentColor;
            visible: x >= 0 && x <= parent.width;
            Rectangle {
                height: 15 * dpiScale;
                width: 18 * dpiScale;
                color: styleAccentColor;
                radius: 3 * dpiScale;
                y: -5 * dpiScale;
                x: -width / 2;

                Rectangle {
                    height: 12 * dpiScale;
                    width: 15 * dpiScale;
                    color: parent.color;
                    radius: 3 * dpiScale;
                    anchors.centerIn: parent;
                    anchors.verticalCenterOffset: 5 * dpiScale;
                    rotation: 45;
                }
                Rectangle {
                    width: 1.5 * dpiScale;
                    color: "#000";
                    height: 6 * dpiScale;
                    radius: width;
                    anchors.horizontalCenter: parent.horizontalCenter;
                    anchors.horizontalCenterOffset: 1 * dpiScale;
                    anchors.bottom: parent.bottom;
                    anchors.bottomMargin: -6 * dpiScale;
                }
            }
        }

        Repeater {
            model: controller.offsets_model;

            TimelineSyncPoint {
                y: (root.fullScreen? 0 : 35) * dpiScale;
                timeline: root;
                org_timestamp_us: timestamp_us;
                position: (timestamp_us + offset_ms * 1000) / (root.durationMs * 1000.0); // TODO: Math.round?
                value: offset_ms;
                unit: qsTr("ms");
                isCalibPoint: false;
                onEdit: (ts_us, val) => {
                    root.editingSyncPoint = true;
                    syncPointSlider.timestamp_us = ts_us;
                    syncPointSlider.from  = val - Math.max(15, Math.abs(val));
                    syncPointSlider.to    = val + Math.max(15, Math.abs(val));
                    syncPointSlider.value = val;
                }
                onRemove: (ts_us) => {
                    root.editingSyncPoint = false;
                    controller.remove_offset(ts_us);
                }
                onZoomIn: (ts_us) => {
                    const start_ts = ts_us - (window.sync.timePerSyncpoint.value * 1000000 / 2) * 1.05;
                    const end_ts   = ts_us + (window.sync.timePerSyncpoint.value * 1000000 / 2) * 1.05;
                    root.visibleAreaLeft  = start_ts / (root.durationMs * 1000.0);
                    root.visibleAreaRight = end_ts   / (root.durationMs * 1000.0);
                    chart.setVScaleToVisibleArea();
                }
            }
        }
        Repeater {
            visible: isCalibrator;
            model: isCalibrator? controller.calib_model : [];

            TimelineSyncPoint {
                y: (root.fullScreen? 0 : 35) * dpiScale;
                timeline: root;
                color: is_forced? "#11d144" : "#17b3f0";
                org_timestamp_us: timestamp_us;
                position: timestamp_us / (root.durationMs * 1000.0); // TODO: Math.round?
                value: sharpness;
                unit: qsTr("px");
                isCalibPoint: true;
                onEdit: (ts_us, val) => {
                    vid.setTimestamp(ts_us / 1000);
                }
                onRemove: (ts_us) => {
                    root.editingSyncPoint = false;
                    controller.remove_calibration_point(ts_us);
                }
            }
        }

        QQC.ScrollBar {
            id: scrollbar;
            hoverEnabled: true;
            visible: !root.fullScreen && size < 1.0;
            active: hovered || pressed;
            orientation: Qt.Horizontal;
            size: root.visibleAreaRight - root.visibleAreaLeft;
            anchors.left: parent.left;
            anchors.right: parent.right;
            anchors.bottom: parent.bottom;
            position: 0;
            onPositionChanged: {
                const diff = root.visibleAreaRight - root.visibleAreaLeft;
                root.visibleAreaLeft = position;
                root.visibleAreaRight = position + diff;
            }
        }
    }

    property bool editingSyncPoint: false;
    property real additionalHeight: editingSyncPoint? 35 : 0;
    Ease on additionalHeight { }

    Row {
        id: row;
        x: 30 * dpiScale;
        width: parent.width - x;
        spacing: 10 * dpiScale;
        height: 35 * dpiScale;
        anchors.bottom: parent.bottom;
        anchors.bottomMargin: 0 * dpiScale;
        visible: opacity > 0;
        opacity: parent.editingSyncPoint? 1 : 0;
        Ease on opacity {}
        Slider {
            id: syncPointSlider;
            property int timestamp_us: 0;
            width: parent.width - syncPointEditField.width - syncPointBtn.width - 30 * dpiScale;
            anchors.verticalCenter: parent.verticalCenter;
            property bool preventChange: false;
            onValueChanged: if (!preventChange) syncPointEditField.value = value;
            unit: qsTr("ms")
        }
        NumberField {
            id: syncPointEditField;

            width: 90 * dpiScale;
            precision: 3;
            unit: qsTr("ms");
            anchors.verticalCenter: parent.verticalCenter;
            property bool preventChange: true;
            onValueChanged: {
                if (preventChange) return;
                syncPointSlider.preventChange = true;
                syncPointSlider.value = value;
                syncPointSlider.preventChange = false;

                controller.set_offset(syncPointSlider.timestamp_us, value);
            }
            Component.onCompleted: {
                preventChange = false;
            }
            onAccepted: {
                controller.set_offset(syncPointSlider.timestamp_us, value);
            }
        }
        Button {
            id: syncPointBtn;
            text: qsTr("Save");
            accent: true;
            height: 25 * dpiScale;
            leftPadding: 8 * dpiScale;
            rightPadding: 8 * dpiScale;
            font.pixelSize: 12 * dpiScale;
            anchors.verticalCenter: parent.verticalCenter;
            onClicked: {
                root.editingSyncPoint = false;
                controller.set_offset(syncPointSlider.timestamp_us, syncPointEditField.value);
            }
        }
    }
    LoaderOverlay { anchors.topMargin: 10 * dpiScale; }

    Item {
        width: parent.width;
        anchors.bottom: parent.bottom;
        ToolTip {
            text: qsTr("%1 to zoom horizontally, %2 to zoom vertically, %3 to pan, double click to reset zoom")
                    .arg("<b>" + qsTr("Scroll") + "</b>")
                    .arg("<b>" + (Qt.platform.os == "osx"? qsTr("Control+Shift+Scroll") : qsTr("Alt+Scroll")) + "</b>")
                    .arg("<b>" + (Qt.platform.os == "osx"? qsTr("Command+Scroll") : qsTr("Ctrl+Scroll")) + "</b>");
            visible: ma.containsMouse;
            delay: 2000;
        }
    }
}
