import QtQuick 2.15
import QtQuick.Dialogs

import "../components/"

MenuItem {
    id: root;
    text: qsTr("Lens profile");
    icon: "lens";

    property real calibWidth: 0;
    property real calibHeight: 0;

    property real videoWidth: 0;
    property real videoHeight: 0;

    FileDialog {
        id: fileDialog;
        property var extensions: ["json"];

        title: qsTr("Choose a lens profile")
        nameFilters: Qt.platform.os == "android"? undefined : [qsTr("Lens profiles") + " (*.json)"];
        onAccepted: loadFile(fileDialog.selectedFile);
    }
    function loadFile(url) {
        if (Qt.platform.os == "android") {
            url = Qt.resolvedUrl("file://" + controller.resolve_android_url(url.toString()));
        }
        controller.load_lens_profile(url.toString());
    }

    Component.onCompleted: {
        let list = [];
        for (const x of lensProfilesList) {
            let fname = x.split("/").pop().replace(".json", "");
            fname = fname.replace("4_3", "4:3").replace("4by3", "4:3").replace("16_9", "16:9").replace("16by9", "16:9");
            fname = fname.replace("2_7K", "2.7k").replace("4K", "4k").replace("5K", "5k");

            list.push(fname.replace(/_/g, " "))
        }
        search.model = list;

        QT_TRANSLATE_NOOP("TableList", "Camera");
        QT_TRANSLATE_NOOP("TableList", "Lens");
        QT_TRANSLATE_NOOP("TableList", "Setting");
        QT_TRANSLATE_NOOP("TableList", "Dimensions");
        QT_TRANSLATE_NOOP("TableList", "Calibrated by");
    }
    Connections {
        target: controller;
        function onLens_profile_loaded(obj) {
            info.model = {
                "Camera":        obj.camera,
                "Lens":          obj.lens,
                "Setting":       obj.camera_setting,
                "Dimensions":    obj.calib_dimension,
                "Calibrated by": obj.calibrated_by
            };
            root.calibWidth  = obj.calib_width;
            root.calibHeight = obj.calib_height;
            const coeffs = obj.coefficients.split(";").map(parseFloat);
            const mtrx = obj.matrix.split(";").map(parseFloat);
            k1.setInitialValue(coeffs[0]);
            k2.setInitialValue(coeffs[1]);
            k3.setInitialValue(coeffs[2]);
            k4.setInitialValue(coeffs[3]);
            fx.setInitialValue(mtrx[0]);
            fy.setInitialValue(mtrx[4]);
        }
    }

    SearchField {
        id: search;
        placeholderText: qsTr("Search...");
        height: 25 * dpiScale;
        width: parent.width;
        topPadding: 5 * dpiScale;
        onSelected: (text, index) => {
            controller.load_lens_profile(lensProfilesList[index]);
        }
    }
    Button {
        text: qsTr("Open file");
        icon.name: "file-empty"
        anchors.horizontalCenter: parent.horizontalCenter;
        onClicked: fileDialog.open();
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
            function setInitialValue(v) {
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
        z: 999;
        anchors.rightMargin: -28 * dpiScale;
        anchors.topMargin: 35 * dpiScale;
        anchors.bottomMargin: -35 * dpiScale;
        extensions: fileDialog.extensions;
        onLoadFile: (url) => root.loadFile(url);
    }
}
