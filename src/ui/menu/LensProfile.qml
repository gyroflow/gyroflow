// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Dialogs

import "../components/"

MenuItem {
    id: root;
    text: qsTr("Lens profile");
    iconName: "lens";
    objectName: "lens";

    property int calibWidth: 0;
    property int calibHeight: 0;

    property int videoWidth: 0;
    property int videoHeight: 0;

    property var lensProfilesList: [];
    property var distortionCoeffs: [];
    property string profileName;

    FileDialog {
        id: fileDialog;
        property var extensions: ["json"];

        title: qsTr("Choose a lens profile")
        nameFilters: Qt.platform.os == "android"? undefined : [qsTr("Lens profiles") + " (*.json)"];
        type: "lens";
        onAccepted: loadFile(fileDialog.selectedFile);
    }
    function loadFile(url: url) {
        if (Qt.platform.os == "android") {
            url = Qt.resolvedUrl("file://" + controller.resolve_android_url(url.toString()));
        }
        controller.load_lens_profile_url(url);
    }

    Component.onCompleted: {
        controller.fetch_profiles_from_github();
        controller.load_profiles();

        QT_TRANSLATE_NOOP("TableList", "Camera");
        QT_TRANSLATE_NOOP("TableList", "Lens");
        QT_TRANSLATE_NOOP("TableList", "Setting");
        QT_TRANSLATE_NOOP("TableList", "Additional info");
        QT_TRANSLATE_NOOP("TableList", "Dimensions");
        QT_TRANSLATE_NOOP("TableList", "Calibrated by");
    }
    Timer {
        id: profilesUpdateTimer;
        interval: 1000;
        onTriggered: controller.load_profiles();
    }
    Connections {
        target: controller;
        function onAll_profiles_loaded(profiles) {
            lensProfilesList = profiles;

            let list = [];
            for (const x of lensProfilesList) {
                list.push(x[0])
            }
            search.model = list;
        }
        function onLens_profiles_updated() {
            profilesUpdateTimer.start();
        }
        function onLens_profile_loaded(json_str: string, filepath: string) {
            if (json_str) {
                const obj = JSON.parse(json_str);
                if (obj) {
                    info.model = {
                        "Camera":          obj.camera_brand + " " + obj.camera_model,
                        "Lens":            obj.lens_model,
                        "Setting":         obj.camera_setting,
                        "Additional info": obj.note,
                        "Dimensions":      obj.calib_dimension.w + "x" + obj.calib_dimension.h,
                        "Calibrated by":   obj.calibrated_by
                    };
                    officialInfo.show = !obj.official;
                    officialInfo.canRate = true;
                    officialInfo.thankYou = false;
                    root.profileName = (filepath || obj.name || "").replace(/^.*?[\/\\]([^\/\\]+?)$/, "$1");

                    if (obj.output_dimension && obj.output_dimension.w > 0 && (obj.calib_dimension.w != obj.output_dimension.w || obj.calib_dimension.h != obj.output_dimension.h)) {
                        Qt.callLater(window.exportSettings.lensProfileLoaded, obj.output_dimension.w, obj.output_dimension.h);
                    }
                    if (+obj.frame_readout_time && Math.abs(+obj.frame_readout_time) > 0) {
                        window.stab.setFrameReadoutTime(obj.frame_readout_time);
                    }
                    if (+obj.gyro_lpf && Math.abs(+obj.gyro_lpf) > 0) {
                        window.motionData.setGyroLpf(obj.gyro_lpf);
                    }
                    if (obj.sync_settings && Object.keys(obj.sync_settings).length > 0) {
                        window.sync.loadGyroflow({
                            synchronization: obj.sync_settings
                        });
                    }

                    const input_horizontal_stretch = obj.input_horizontal_stretch > 0.01? obj.input_horizontal_stretch : 1.0;
                    const input_vertical_stretch   = obj.input_vertical_stretch   > 0.01? obj.input_vertical_stretch   : 1.0;

                    root.calibWidth  = obj.calib_dimension.w / input_horizontal_stretch;
                    root.calibHeight = obj.calib_dimension.h / input_vertical_stretch;
                    const coeffs = obj.fisheye_params.distortion_coeffs;
                    root.distortionCoeffs = coeffs;
                    const mtrx = obj.fisheye_params.camera_matrix;
                    k1.setInitialValue(coeffs[0]);
                    k2.setInitialValue(coeffs[1]);
                    k3.setInitialValue(coeffs[2]);
                    k4.setInitialValue(coeffs[3]);
                    fx.setInitialValue(mtrx[0][0]);
                    fy.setInitialValue(mtrx[1][1]);
                    cx.setInitialValue(mtrx[0][2]);
                    cy.setInitialValue(mtrx[1][2]);
                }
            }
        }
    }

    SearchField {
        id: search;
        placeholderText: qsTr("Search...");
        height: 25 * dpiScale;
        width: parent.width;
        popup.width: width * 1.7;
        topPadding: 5 * dpiScale;
        onSelected: (text, index) => {
            controller.load_lens_profile(lensProfilesList[index][1]);
        }
    }
    Row {
        anchors.horizontalCenter: parent.horizontalCenter;
        spacing: 10 * dpiScale;
        Button {
            text: qsTr("Open file");
            iconName: "file-empty"
            onClicked: fileDialog.open2();
        }
        Button {
            text: qsTr("Create new");
            iconName: "plus";
            icon.width: 15 * dpiScale;
            icon.height: 15 * dpiScale;
            property var calibratorWnd: null;
            onClicked: {
                if (!calibratorWnd) {
                    ui_tools.init_calibrator();
                    calibratorWnd = Qt.createComponent("../Calibrator.qml").createObject(main_window)
                    calibratorWnd.show();
                    calibratorWnd.closing.connect(function(e) {
                        calibratorWnd = null;
                    })
                }
            }
        }
    }

    InfoMessageSmall {
        id: officialInfo;
        type: InfoMessage.Warning;
        show: false;
        property bool canRate: true;
        property bool thankYou: false;
        text: qsTr("This lens profile is unofficial, we can't guarantee it's correctness. Use at your own risk.") + (canRate? "<br>" +
              qsTr("Rate this profile: [Good] | [Bad]")
              .replace(/\[(.*?)\]/, "<a href=\"#good\">$1</a>")
              .replace(/\[(.*?)\]/, "<a href=\"#bad\">$1</a>") : (thankYou? "<br>" + qsTr("Thank you for rating this profile.") : ""));

        MouseArea {
            anchors.fill: parent;
            cursorShape: parent.t.hoveredLink? Qt.PointingHandCursor : Qt.ArrowCursor;
            acceptedButtons: Qt.NoButton;
        }
        Connections {
            target: officialInfo.t;
            function onLinkActivated(link: url) {
                const str = root.profileName + "|" + root.distortionCoeffs.join("|");
                controller.rate_profile(str, link === "#good");
                officialInfo.thankYou = true;
                officialInfo.canRate = false;
                tyTimer.start();
            }
        }
        Timer {
            id: tyTimer;
            interval: 5000;
            onTriggered: officialInfo.thankYou = false;
        }
    }

    InfoMessageSmall {
        type: lensRatio != videoRatio? InfoMessage.Error : InfoMessage.Warning;
        show: root.calibWidth > 0 && root.videoWidth > 0 && (root.calibWidth != root.videoWidth || root.calibHeight != root.videoHeight);
        property string lensRatio: (root.calibWidth / Math.max(1, root.calibHeight)).toFixed(5);
        property string videoRatio: (root.videoWidth / Math.max(1, root.videoHeight)).toFixed(5);
        text: lensRatio != videoRatio? qsTr("Lens profile aspect ratio doesn't match the file aspect ratio. The result will not look correct.") :
                                       qsTr("Lens profile dimensions don't match the file dimensions. The result may not look correct.");
    }

    TableList {
        id: info;
        model: ({ })
    }
    LinkButton {
        visible: Object.keys(info.model).length > 0;
        text: qsTr("Adjust parameters");
        anchors.horizontalCenter: parent.horizontalCenter;
        onClicked: adjust.opened = !adjust.opened;
    }
    Column {
        spacing: parent.spacing;
        id: adjust;
        property bool opened: false;
        width: parent.width;
        visible: opacity > 0;
        opacity: opened? 1 : 0;
        height: opened? implicitHeight : -10 * dpiScale;
        Ease on opacity { }
        Ease on height { }

        component SmallNumberField: NumberField {
            property bool preventChange2: true;
            width: parent.width / 2;
            precision: 12;
            property string param: "  ";
            tooltip: param[0] + "<font size=\"1\">" + param[1] + "</font>"
            font.pixelSize: 11 * dpiScale;
            onValueChanged: {
                if (!preventChange2) controller.set_lens_param(param, value);
            }
            function setInitialValue(v: real) {
                preventChange2 = true;
                value = v;
                preventChange2 = false;
            }
        }

        Label {
            text: qsTr("Pixel focal length");

            Row {
                spacing: 4 * dpiScale;
                width: parent.width;
                SmallNumberField { id: fx; param: "fx"; }
                SmallNumberField { id: fy; param: "fy"; }
            }
        }
        Label {
            text: qsTr("Focal center");

            Row {
                spacing: 4 * dpiScale;
                width: parent.width;
                SmallNumberField { id: cx; param: "cx"; }
                SmallNumberField { id: cy; param: "cy"; }
            }
        }
        Label {
            text: qsTr("Distortion coefficients");

            Column {
                spacing: 4 * dpiScale;
                width: parent.width;
                Row {
                    spacing: 4 * dpiScale;
                    width: parent.width;
                    SmallNumberField { id: k1; param: "k1"; precision: 16; }
                    SmallNumberField { id: k2; param: "k2"; precision: 16; }
                }
                Row {
                    spacing: 4 * dpiScale;
                    width: parent.width;
                    SmallNumberField { id: k3; param: "k3"; precision: 16; }
                    SmallNumberField { id: k4; param: "k4"; precision: 16; }
                }
            }
        }
    }

    DropTarget {
        parent: root.innerItem;
        color: styleBackground2;
        z: 999;
        anchors.rightMargin: -28 * dpiScale;
        anchors.topMargin: 35 * dpiScale;
        anchors.bottomMargin: -35 * dpiScale;
        extensions: fileDialog.extensions;
        onLoadFile: (url) => root.loadFile(url);
    }
}
