// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Gyroflow

import QtQuick
import QtQuick.Controls as QQC

import "../components/"

MenuItem {
    id: root;
    text: qsTr("Color grading");
    iconName: "color";
    objectName: "colorGrading";
    innerItem.enabled: window.videoArea.vid.loaded;

    Item {
        id: sett;
        property alias basicEnabled: basicEnabled.checked;
        property alias creativeEnabled: creativeEnabled.checked;
        property alias temperature: temperature.value;
        property alias tint: tint.value;
        property alias basicSaturation: basicSaturation.value;
        property alias exposure: exposure.value;
        property alias contrast: contrast.value;
        property alias highlights: highlights.value;
        property alias shadows: shadows.value;
        property alias whites: whites.value;
        property alias blacks: blacks.value;
        property alias fadedFilm: fadedFilm.value;
        property alias vibrance: vibrance.value;
        property alias creativeSaturation: creativeSaturation.value;
        property alias lutEnabled: lutEnabled.checked;
        property alias lutStrength: lutStrength.value;
        Component.onCompleted: settings.init(sett);
        function propChanged() { settings.propChanged(sett); }
    }

    CheckBox {
        id: basicEnabled;
        text: qsTr("Enable basic correction");
        checked: false;
        onCheckedChanged: { controller.set_cg_basic_enabled(checked); sett.propChanged(); }
    }

    FileDialog {
        id: lutFileDialog;
        type: "lut";
        nameFilters: [qsTr("LUT files") + " (*.cube *.CUBE)"];
        onAccepted: {
            lutPathField.text = (selectedFile + "").split('/').pop();
            controller.set_cg_lut_path(selectedFile.toString());
            lutEnabled.checked = true;
            sett.propChanged();
        }
    }

    Label {
        text: qsTr("LUT");
        width: parent.width;
        Row {
            width: parent.width;
            spacing: 5 * dpiScale;
            Button {
                text: qsTr("Select LUT");
                onClicked: lutFileDialog.open2();
            }
            BasicText {
                id: lutPathField;
                text: qsTr("None");
                width: parent.width - 100 * dpiScale;
                elide: Text.ElideMiddle;
                anchors.verticalCenter: parent.verticalCenter;
            }
        }
    }
    CheckBox {
        id: lutEnabled;
        text: qsTr("Enable LUT");
        checked: false;
        onCheckedChanged: { controller.set_cg_lut_enabled(checked); sett.propChanged(); }
    }
    Label {
        text: qsTr("LUT strength");
        width: parent.width;
        SliderWithField {
            id: lutStrength;
            from: 0; to: 100; value: 100; defaultValue: 100; precision: 0; width: parent.width;
            onValueChanged: { controller.set_cg_lut_strength(value / 100.0); sett.propChanged(); }
        }
    }

    Hr { }

    BasicText { text: qsTr("Color"); }

    Label {
        text: qsTr("Temperature");
        width: parent.width;
        SliderWithField {
            id: temperature;
            from: -100; to: 100; value: 0; defaultValue: 0; precision: 0; width: parent.width;
            onValueChanged: { controller.set_cg_temperature(value / 100.0); sett.propChanged(); }
        }
    }
    Label {
        text: qsTr("Tint");
        width: parent.width;
        SliderWithField {
            id: tint;
            from: -100; to: 100; value: 0; defaultValue: 0; precision: 0; width: parent.width;
            onValueChanged: { controller.set_cg_tint(value / 100.0); sett.propChanged(); }
        }
    }
    Label {
        text: qsTr("Saturation");
        width: parent.width;
        SliderWithField {
            id: basicSaturation;
            from: 0; to: 200; value: 100; defaultValue: 100; precision: 0; width: parent.width;
            onValueChanged: { controller.set_cg_basic_saturation(value / 100.0); sett.propChanged(); }
        }
    }

    BasicText { text: qsTr("Light"); }

    Label {
        text: qsTr("Exposure");
        width: parent.width;
        SliderWithField {
            id: exposure;
            from: -100; to: 100; value: 0; defaultValue: 0; precision: 0; width: parent.width;
            onValueChanged: { controller.set_cg_exposure(value / 100.0); sett.propChanged(); }
        }
    }
    Label {
        text: qsTr("Contrast");
        width: parent.width;
        SliderWithField {
            id: contrast;
            from: -100; to: 100; value: 0; defaultValue: 0; precision: 0; width: parent.width;
            onValueChanged: { controller.set_cg_contrast(value / 100.0); sett.propChanged(); }
        }
    }
    Label {
        text: qsTr("Highlights");
        width: parent.width;
        SliderWithField {
            id: highlights;
            from: -100; to: 100; value: 0; defaultValue: 0; precision: 0; width: parent.width;
            onValueChanged: { controller.set_cg_highlights(value / 100.0); sett.propChanged(); }
        }
    }
    Label {
        text: qsTr("Shadows");
        width: parent.width;
        SliderWithField {
            id: shadows;
            from: -100; to: 100; value: 0; defaultValue: 0; precision: 0; width: parent.width;
            onValueChanged: { controller.set_cg_shadows(value / 100.0); sett.propChanged(); }
        }
    }
    Label {
        text: qsTr("Whites");
        width: parent.width;
        SliderWithField {
            id: whites;
            from: -100; to: 100; value: 0; defaultValue: 0; precision: 0; width: parent.width;
            onValueChanged: { controller.set_cg_whites(value / 100.0); sett.propChanged(); }
        }
    }
    Label {
        text: qsTr("Blacks");
        width: parent.width;
        SliderWithField {
            id: blacks;
            from: -100; to: 100; value: 0; defaultValue: 0; precision: 0; width: parent.width;
            onValueChanged: { controller.set_cg_blacks(value / 100.0); sett.propChanged(); }
        }
    }

    Button {
        text: qsTr("Reset");
        anchors.right: parent.right;
        onClicked: {
            controller.reset_color_grading();
            temperature.value = 0; tint.value = 0; basicSaturation.value = 100;
            exposure.value = 0; contrast.value = 0; highlights.value = 0;
            shadows.value = 0; whites.value = 0; blacks.value = 0;
            fadedFilm.value = 0; vibrance.value = 0; creativeSaturation.value = 100;
            lutEnabled.checked = false; lutStrength.value = 100; lutPathField.text = qsTr("None");
            sett.propChanged();
        }
    }

    Hr { }

    CheckBox {
        id: creativeEnabled;
        text: qsTr("Enable creative");
        checked: false;
        onCheckedChanged: { controller.set_cg_creative_enabled(checked); sett.propChanged(); }
    }

    BasicText { text: qsTr("Adjustments"); }

    Label {
        text: qsTr("Faded film");
        width: parent.width;
        SliderWithField {
            id: fadedFilm;
            from: 0; to: 100; value: 0; defaultValue: 0; precision: 0; width: parent.width;
            onValueChanged: { controller.set_cg_faded_film(value / 100.0); sett.propChanged(); }
        }
    }
    Label {
        text: qsTr("Vibrance");
        width: parent.width;
        SliderWithField {
            id: vibrance;
            from: -100; to: 100; value: 0; defaultValue: 0; precision: 0; width: parent.width;
            onValueChanged: { controller.set_cg_vibrance(value / 100.0); sett.propChanged(); }
        }
    }
    Label {
        text: qsTr("Saturation");
        width: parent.width;
        SliderWithField {
            id: creativeSaturation;
            from: 0; to: 200; value: 100; defaultValue: 100; precision: 0; width: parent.width;
            onValueChanged: { controller.set_cg_creative_saturation(value / 100.0); sett.propChanged(); }
        }
    }
}
