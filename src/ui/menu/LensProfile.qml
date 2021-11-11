import QtQuick 2.15
import QtQuick.Dialogs

import "../components/"

MenuItem {
    id: root;
    text: qsTr("Lens profile");
    icon: "lens";

    FileDialog {
        id: fileDialog;
        property var extensions: ["json"];

        title: qsTr("Choose a lens profile")
        nameFilters: [qsTr("Lens profiles") + " (*.json)"];
        onAccepted: controller.load_lens_profile((fileDialog.selectedFile + "").replace("file:///", ""));
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
    }
    Connections {
        target: controller;
        function onLens_profile_loaded(obj) {
            // TODO: translations
            info.model = {
                "Camera": obj.camera,
                "Lens": obj.lens,
                "Setting": obj.camera_setting,
                "Dimensions": obj.calib_dimension,
                "Calibrated by": obj.calibrated_by
            };
            const coeffs = obj.coefficients.split(";").map(parseFloat);
            const mtrx = obj.matrix.split(";").map(parseFloat);
            d0.value = coeffs[0];
            d1.value = coeffs[1];
            d2.value = coeffs[2];
            d3.value = coeffs[3];
            fc0.value = mtrx[0]
            fc1.value = mtrx[4]
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
    TableList {
        id: info;
        model: ({ })
    }
    LinkButton {
        visible: Object.keys(info.model).length > 0;
        text: qsTr("Adjust parameters");
        //icon.name: "pencil"
        //icon.height: 14 * dpiScale;
        //icon.width: 14 * dpiScale;
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
        //clip: true;

        Label {
            text: qsTr("Focal center");

            Column {
                spacing: 4 * dpiScale;
                width: parent.width;
                NumberField { id: fc0; width: parent.width; precision: 12; }
                NumberField { id: fc1; width: parent.width; precision: 12; }
            }
        }
        /*
        Label {
            text: qsTr("Focal center");

            Column {
                spacing: 4 * dpiScale;
                width: parent.width;
                NumberField { width: parent.width; precision: 6; }
                NumberField { width: parent.width; precision: 6; }
            }
        }*/
        Label {
            text: qsTr("Distortion coefficients");

            Column {
                spacing: 4 * dpiScale;
                width: parent.width;
                NumberField { id: d0; width: parent.width; precision: 16; }
                NumberField { id: d1; width: parent.width; precision: 16; }
                NumberField { id: d2; width: parent.width; precision: 16; }
                NumberField { id: d3; width: parent.width; precision: 16; }
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
        onLoadFile: (url) => {
            const path = url.toString().replace("file:///", "");
            onAccepted: controller.load_lens_profile(path);
        }
    }
}
