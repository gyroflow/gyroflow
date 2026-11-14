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
    // The last estimate (`Controller::find_lens_from_video`'s `estimated`), and whether the file states a readout
    // time of its own
    property var result: null;
    property bool fileReadout: false;

    // The project's video into the calibrator. Whatever is open there goes - a calibration video, with what was
    // detected in it - so that is asked first
    function showProjectVideo(): void {
        if (!hasProjectVideo || sameVideo) return;
        if (!calibrator_window.anyFileLoaded) { calibrator_window.loadFile(projectVideo); return; }
        messageBox(Modal.Warning, qsTr("The video that is open in the calibrator will be closed, with the calibration made on it.\nShow the project's video here?"), [
            { text: qsTr("Yes"), accent: true, clicked: () => calibrator_window.loadFile(root.projectVideo) },
            { text: qsTr("No") },
        ]);
    }
    function estimate(): void {
        root.result = null;
        root.waiting = true;
        const position = fragment.currentIndex == 1? String(calibrator_window.videoArea.timeline.position) : "";
        const name = settings.value("calibratedBy", "") || main_controller.get_username();
        main_controller.find_lens_from_video(position, true, underWater.checked? "water" : "air", name);
    }
    function useEstimate(): void {
        if (!root.result) return;
        window.lensProfile.useFoundLens("estimated", root.result);
        if (measuredReadout.visible && measuredReadout.checked) window.lensProfile.applyFound("estimated", root.result, true);
        // Done here: the calibrator closes without asking
        calibrator_window.closeConfirmationModal = true;
        calibrator_window.close();
    }

    // The medium a search on this video found, when it fits and was sure of it, or else the one the project is set to.
    // What was found on another video isn't this one's: the lens panel drops it when a video is loaded
    function syncMedium(): void {
        const found = window.lensProfile.lastFound;
        underWater.checked = found && found.apply_medium? !!found.under_water : window.lensProfile.underWater;
    }
    Component.onCompleted: syncMedium();
    // An estimate is the lens of the video it was made from: with another one in the project it's nobody's
    onProjectVideoChanged: root.result = null;
    // With nothing open in the calibrator there's nothing to lose: the project's video is shown. Over a calibration
    // it's the button's to do, which asks
    onVisibleChanged: if (visible) { syncMedium(); if (!calibrator_window.anyFileLoaded) showProjectVideo(); }
    // The calibrator closed: the estimate it asked for has nobody to report to
    Component.onDestruction: if (root.waiting) main_controller.cancel_lens_from_video();

    Connections {
        target: main_controller;
        function onLens_from_video_progress(progress: real, status: string): void {
            if (!root.waiting) return;
            const loader = calibrator_window.videoArea.videoLoader;
            loader.active = progress < 1;
            loader.progress = progress < 1? progress : -1;
            loader.additional = "";
            loader.text = progress < 1? (status == "estimating"? qsTr("Estimating the lens %1...")
                                        : status == "checking"? qsTr("Checking it on the clip %1...")
                                        : qsTr("Following the picture %1...")) : "";
            loader.cancelable = true;
        }
        function onLens_from_video_result(json: string): void {
            const r = JSON.parse(json);
            // The lens panel's search reports to the same controller: that one isn't this panel's
            if (!r || r.mode != "estimate" || !root.waiting) return;
            root.waiting = false;
            if (r.cancelled) return;
            if (r.failed) {
                messageBox(Modal.Error, qsTr("The lens could not be estimated from this video: %1").arg(window.lensProfile.findLensFailure(r)), [ { text: qsTr("Ok"), accent: true } ]);
                return;
            }
            root.fileReadout = !!r.readout_from_file;
            root.result = r.estimated || null;
        }
    }
    // The loader's Cancel goes to the calibrator's controller: the work is the project's, and only this work is stopped
    Connections {
        target: calibrator_window.videoArea.videoLoader;
        function onCancel(): void { if (root.waiting) main_controller.cancel_lens_from_video(); }
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

    Button {
        text: qsTr("Estimate the lens");
        accent: true;
        iconName: "search";
        enabled: root.sameVideo && !root.busy;
        anchors.horizontalCenter: parent.horizontalCenter;
        tooltip: qsTr("About a minute.");
        onClicked: root.estimate();
    }
    BasicText {
        width: parent.width;
        wrapMode: Text.WordWrap;
        font.pixelSize: 12 * dpiScale;
        text: qsTr("The lens profile and the rolling shutter readout time, from the picture of the video alone. Try \"Auto\" next to the lens profile search first: when a known profile of the camera fits the video, there is nothing to estimate.");
    }

    Label {
        position: Label.LeftPosition;
        text: qsTr("Fragment");
        tooltip: qsTr("The stretch of the clip the lens is estimated from: up to 300 frames from there.");

        LinkButton {
            id: fragment;
            property var options: [QT_TRANSLATE_NOOP("Popup", "From the start of the clip"), QT_TRANSLATE_NOOP("Popup", "From the playhead")];
            property int currentIndex: 0;
            text: qsTranslate("Popup", options[currentIndex]) + (currentIndex == 1? " (" + calibrator_window.videoArea.timeline.timeAtPosition(calibrator_window.videoArea.timeline.position) + ")" : "");
            leftPadding: 6 * dpiScale;
            rightPadding: 6 * dpiScale;
            onClicked: if (fragmentPopup.visible) { fragmentPopup.close(); } else { fragmentPopup.open(); }
            Popup {
                id: fragmentPopup;
                model: parent.options;
                currentIndex: parent.currentIndex;
                width: maxItemWidth + 10 * dpiScale;
                y: parent.height;
                itemHeight: 25 * dpiScale;
                font.pixelSize: 11 * dpiScale;
                onClicked: i => fragment.currentIndex = i;
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

    // The estimate, judged the way a known profile is: on parts of the clip it wasn't made from
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
                  (typeof root.result.readout_ms === "number"? qsTr("Focal length %1 px, readout time %2 ms.").arg(window.lensProfile.num(root.result.fx, 0)).arg(window.lensProfile.num(Math.abs(root.result.readout_ms), 1))
                                                             : qsTr("Focal length %1 px.").arg(window.lensProfile.num(root.result.fx, 0))) + "<br>" +
                  qsTr("Points the estimate wasn't made from are %1 px off on average (%2 px with the profile it started from).").arg(window.lensProfile.num(root.result.error, 1)).arg(window.lensProfile.num(root.result.error_before, 1)) : "";
        }
        BasicText {
            width: parent.width;
            wrapMode: Text.WordWrap;
            textFormat: Text.StyledText;
            text: !root.result? "" : root.result.match? window.lensProfile.matchText(root.result.match, root.result.checked_clip || "")
                : "<small>" + qsTr("Not checked on other parts of the clip: it has none apart from the one the lens was estimated from.") + "</small>";
        }
        InfoMessageSmall {
            type: InfoMessage.Warning;
            show: !!(root.result && root.result.doubts && root.result.doubts.length > 0);
            text: show? qsTr("Not to be trusted: %1. Try another fragment.").arg(window.lensProfile.findLensDoubts(root.result.doubts)) : "";
        }
        // The file states its own readout time, and that one is kept unless the user says otherwise
        CheckBox {
            id: measuredReadout;
            visible: root.fileReadout && !!root.result && typeof root.result.readout_ms === "number";
            text: qsTr("Use the measured readout time in place of the one the file states");
            checked: false;
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
