// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Anton Belkin

import QtQuick
import QtQuick.Dialogs

import "../components/"

// "From the video" in the lens calibrator: the lens of the project's video estimated from its picture, with no
// calibration target (`core::lens_from_video`). It runs on the project's controller (`main_controller`), which has the
// video, its camera and the lens profile database; the calibrator shows the same video, for its playhead to pick the
// fragment with. The lens it finds goes into the project only when the user takes it
MenuItem {
    id: root;
    text: qsTr("Lens from the video");
    iconName: "lens";
    objectName: "lensfromvideo";

    readonly property url projectVideo: window.videoArea.loadedFileUrl;
    readonly property bool hasProjectVideo: window.videoArea.vid.loaded && projectVideo.toString() != "";
    readonly property bool sameVideo: hasProjectVideo && calibrator_window.videoArea.loadedFileUrl.toString() == projectVideo.toString();
    readonly property bool busy: main_controller.lens_from_video_in_progress;
    // A request of this panel is running: the lens panel's search reports to the same controller
    property bool waiting: false;
    // The last estimate (`Controller::find_lens_from_video`'s `estimated`)
    property var result: null;

    function showProjectVideo(): void {
        if (hasProjectVideo && !sameVideo) calibrator_window.loadFile(projectVideo);
    }
    function estimate(): void {
        root.result = null;
        root.waiting = true;
        const fragment = fromPlayhead.checked? String(calibrator_window.videoArea.timeline.position) : "";
        const name = settings.value("calibratedBy", "") || main_controller.get_username();
        main_controller.find_lens_from_video(fragment, true, underWater.checked? "water" : "air", name);
    }
    function useEstimate(): void {
        if (!root.result) return;
        window.lensProfile.useFoundLens("estimated", root.result);
        // Done here: the calibrator closes without asking
        calibrator_window.closeConfirmationModal = true;
        calibrator_window.close();
    }

    // The medium the lens panel's search found, or the one the project is set to
    function syncMedium(): void {
        const found = window.lensProfile.lastFound;
        underWater.checked = found && found.medium_sure && found.media_tried > 1? !!found.under_water : window.lensProfile.underWater;
    }
    Component.onCompleted: syncMedium();
    onVisibleChanged: if (visible) { syncMedium(); showProjectVideo(); }

    Connections {
        target: main_controller;
        function onLens_from_video_progress(progress: real, status: string): void {
            if (!root.waiting) return;
            const loader = calibrator_window.videoArea.videoLoader;
            loader.active = progress < 1;
            loader.progress = progress < 1? progress : -1;
            loader.additional = "";
            loader.text = progress < 1? (status == "estimating the lens"? qsTr("Estimating the lens %1...")
                                        : status == "checking it on the clip"? qsTr("Checking it on the clip %1...")
                                        : qsTr("Following the picture %1...")) : "";
            loader.cancelable = true;
        }
        function onLens_from_video_result(json: string): void {
            if (!root.waiting) return;
            root.waiting = false;
            const r = JSON.parse(json);
            if (!r || r.cancelled) return;
            if (r.failed) {
                messageBox(Modal.Error, qsTr("The lens could not be estimated from this video: %1").arg(r.failed), [ { text: qsTr("Ok"), accent: true } ]);
                return;
            }
            root.result = r.estimated || null;
        }
    }
    // The loader's Cancel goes to the calibrator's controller: the work is the project's
    Connections {
        target: calibrator_window.videoArea.videoLoader;
        function onCancel(): void { if (root.waiting) main_controller.cancel_current_operation(); }
    }

    InfoMessageSmall {
        type: InfoMessage.Warning;
        show: !root.hasProjectVideo;
        text: qsTr("Open the video in the main window first: the lens is found for the video of the project.");
    }
    Button {
        text: qsTr("Show the project's video here");
        visible: root.hasProjectVideo && !root.sameVideo;
        anchors.horizontalCenter: parent.horizontalCenter;
        onClicked: root.showProjectVideo();
    }

    BasicText {
        width: parent.width;
        wrapMode: Text.WordWrap;
        font.pixelSize: 12 * dpiScale;
        text: qsTr("The lens profile and the rolling shutter readout time, from the picture of the video alone. Try \"Auto\" next to the lens profile search first: when a known profile of the camera fits the video, there is nothing to estimate.");
    }

    Label {
        text: qsTr("Fragment");
        Column {
            width: parent.width;
            RadioButton {
                id: fromStart;
                text: qsTr("From the start of the clip");
                checked: true;
            }
            RadioButton {
                id: fromPlayhead;
                text: qsTr("From the playhead (%1)").arg(calibrator_window.videoArea.timeline.timeAtPosition(calibrator_window.videoArea.timeline.position));
            }
        }
    }
    BasicText {
        width: parent.width;
        wrapMode: Text.WordWrap;
        textFormat: Text.StyledText;
        font.pixelSize: 11.5 * dpiScale;
        color: styleTextColor;
        opacity: 0.8;
        text: qsTr("A good fragment, 3 to 10 seconds long:") + "<br>" +
              qsTr("• the camera moves through space while it shakes - walking, swimming, side to side and turning, the more the better - not only turning on the spot;") + "<br>" +
              qsTr("• the scene stands still and has detail all the way into the corners;") + "<br>" +
              qsTr("• few things move on their own, and no particles drifting in the water;") + "<br>" +
              qsTr("• not much motion blur.");
    }

    CheckBox {
        id: underWater;
        text: qsTr("Shot under water, behind a flat port");
        tooltip: qsTr("The picture alone can't tell water from air when nothing is known of the lens: the refraction and the focal length trade off exactly. The profile found is always the lens in air, and the refraction is applied on top of it.");
    }

    Button {
        text: qsTr("Estimate the lens");
        accent: true;
        iconName: "search";
        enabled: root.sameVideo && !root.busy;
        anchors.horizontalCenter: parent.horizontalCenter;
        onClicked: root.estimate();
    }
    BasicText {
        width: parent.width;
        horizontalAlignment: Text.AlignHCenter;
        font.pixelSize: 11.5 * dpiScale;
        opacity: 0.8;
        text: qsTr("About a minute.");
    }

    // The estimate, judged the way a known profile is: on the start, middle and end of the clip
    Column {
        width: parent.width;
        spacing: 5 * dpiScale;
        visible: root.result != null;
        Hr { }
        BasicText {
            width: parent.width;
            wrapMode: Text.WordWrap;
            textFormat: Text.StyledText;
            text: root.result? "<b>" + qsTr("Estimated lens") + "</b>" + (root.result.under_water? " (" + qsTr("under water, the lens in air") + ")" : "") + "<br>" +
                  qsTr("Focal length %1 px, readout time %2 ms.").arg(window.lensProfile.num(root.result.fx, 0)).arg(window.lensProfile.num(root.result.readout_ms, 1)) + "<br>" +
                  qsTr("Points the estimate wasn't made from are %1 px off on average (%2 px with the profile it started from).").arg(window.lensProfile.num(root.result.error, 1)).arg(window.lensProfile.num(root.result.error_before, 1)) : "";
        }
        BasicText {
            width: parent.width;
            wrapMode: Text.WordWrap;
            textFormat: Text.StyledText;
            text: root.result? window.lensProfile.matchText(root.result.match) : "";
        }
        InfoMessageSmall {
            type: InfoMessage.Warning;
            show: !!(root.result && root.result.doubtful);
            text: root.result && root.result.doubtful? qsTr("Not to be trusted: %1. Try another fragment.").arg(root.result.doubtful) : "";
        }
        Row {
            anchors.horizontalCenter: parent.horizontalCenter;
            spacing: 10 * dpiScale;
            Button {
                text: qsTr("Use in the project");
                accent: true;
                enabled: !root.busy;
                onClicked: root.useEstimate();
            }
            LinkButton {
                text: qsTr("Save to file");
                anchors.verticalCenter: parent.verticalCenter;
                onClicked: saveDialog.open2();
            }
        }
    }

    FileDialog {
        id: saveDialog;
        fileMode: FileDialog.SaveFile;
        defaultSuffix: "json";
        title: qsTr("Export lens profile");
        nameFilters: [qsTr("Lens profiles") + " (*.json)"];
        type: "output-preset";
        onAccepted: {
            if (main_controller.save_found_lens("estimated", selectedFile))
                calibrator_window.showNotification(Modal.Info, qsTr("Lens profile exported to %1.").arg("<b>" + filesystem.display_url(selectedFile) + "</b>"));
        }
    }
}
