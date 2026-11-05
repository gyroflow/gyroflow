import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15
import "." 

ColumnLayout {
    id: root
    spacing: 10
    Layout.fillWidth: true

    property bool isCalibrator: false
    property string manualBrand: ""
    property string manualModel: ""
    property string manualLens: ""

    // 1. Camera Brand Selection
    RowLayout {
        Layout.fillWidth: true
        spacing: 8

        Label {
            text: qsTr("Camera Brand:")
            Layout.preferredWidth: 120
        }

        ComboBox {
            id: brandCombo
            Layout.fillWidth: true
            model: LensProfileWrapper.brands
            currentIndex: -1
            displayText: currentIndex === -1 ? qsTr("Select Camera Brand...") : currentText
            onActivated: {
                if (currentText === "Other") {
                    brandManualInput.visible = true;
                } else {
                    brandManualInput.visible = false;
                    LensProfileWrapper.selectBrand(currentText);
                }
            }
        }
    }

    TextField {
        id: brandManualInput
        visible: false
        Layout.fillWidth: true
        placeholderText: qsTr("Type camera brand manually...")
        onTextChanged: {
            root.manualBrand = text;
            LensProfileWrapper.selectBrand(text);
        }
    }

    // 2. Camera Model Selection
    RowLayout {
        Layout.fillWidth: true
        spacing: 8

        Label {
            text: qsTr("Camera Model:")
            Layout.preferredWidth: 120
        }

        ComboBox {
            id: modelCombo
            Layout.fillWidth: true
            model: LensProfileWrapper.models
            currentIndex: -1
            displayText: currentIndex === -1 ? qsTr("Select Camera Model...") : currentText
            onActivated: {
                if (currentText === "Other") {
                    modelManualInput.visible = true;
                } else {
                    modelManualInput.visible = false;
                    LensProfileWrapper.selectModel(currentText);
                }
            }
        }
    }

    TextField {
        id: modelManualInput
        visible: false
        Layout.fillWidth: true
        placeholderText: qsTr("Type camera model manually...")
        onTextChanged: {
            root.manualModel = text;
            LensProfileWrapper.selectModel(text);
        }
    }

    // Compatible Cameras Suggestion Info
    Rectangle {
        id: compBanner
        Layout.fillWidth: true
        visible: LensProfileWrapper.compatible_cameras.length > 0
        implicitHeight: compLabel.implicitHeight + 16
        color: "#2a2d34"
        radius: 4

        RowLayout {
            anchors.fill: parent
            anchors.margins: 8
            spacing: 8
            Label {
                text: "💡"
                font.pixelSize: 14
            }
            Label {
                id: compLabel
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                color: "#9ad1d4"
                text: qsTr("Compatible camera bodies sharing this mount/format: ") +
                      LensProfileWrapper.compatible_cameras.join(", ")
            }
        }
    }

    // 3. Lens Selection
    RowLayout {
        Layout.fillWidth: true
        spacing: 8

        Label {
            text: qsTr("Lens Model:")
            Layout.preferredWidth: 120
        }

        ComboBox {
            id: lensCombo
            Layout.fillWidth: true
            model: LensProfileWrapper.lenses
            editable: true
            displayText: currentText === "" ? qsTr("Select or type lens model...") : currentText
            onEditTextChanged: {
                LensProfileWrapper.selectLens(editText);
            }
            onActivated: {
                LensProfileWrapper.selectLens(currentText);
            }
        }
    }

    // 4. Focal Length (Crucial for zoom lenses)
    RowLayout {
        Layout.fillWidth: true
        spacing: 8

        Label {
            text: qsTr("Focal Length (mm):")
            Layout.preferredWidth: 120
        }

        SpinBox {
            id: focalSpin
            Layout.fillWidth: true
            from: 1
            to: 1200
            stepSize: 1
            editable: true
            value: 24
            onValueChanged: {
                LensProfileWrapper.setFocalLength(value);
            }
        }
    }

    // Validation Status Banner (Enforces strict rules in Calibrator)
    Rectangle {
        Layout.fillWidth: true
        visible: root.isCalibrator && !LensProfileWrapper.can_submit
        implicitHeight: valLabel.implicitHeight + 16
        color: "#4a1e1e"
        radius: 4

        RowLayout {
            anchors.fill: parent
            anchors.margins: 8
            spacing: 8
            Label {
                text: "⚠️"
                font.pixelSize: 14
            }
            Label {
                id: valLabel
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                color: "#ff8b8b"
                text: qsTr("Submission Requirement: ") + LensProfileWrapper.validation_error
            }
        }
    }

    // 5. Review Switcher / A-B Toggle for Reviewing Profiles
    GroupBox {
        title: qsTr("Profile Review & Comparison")
        Layout.fillWidth: true
        visible: LensProfileWrapper.review_entries.length > 1

        ColumnLayout {
            anchors.fill: parent
            spacing: 8

            RowLayout {
                Layout.fillWidth: true
                spacing: 8

                Button {
                    text: "◀ " + qsTr("Previous Profile")
                    onClicked: LensProfileWrapper.prevReviewProfile()
                }

                Label {
                    Layout.fillWidth: true
                    horizontalAlignment: Text.AlignHCenter
                    font.bold: true
                    text: LensProfileWrapper.active_profile_name
                }

                Button {
                    text: qsTr("Next Profile") + " ▶"
                    onClicked: LensProfileWrapper.nextReviewProfile()
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 8

                Label {
                    Layout.fillWidth: true
                    color: "#a0a0a0"
                    text: qsTr("Compare undistortion quality across submitted profiles. Discard inaccurate ones.")
                }

                Button {
                    text: qsTr("Mark Bad / Remove")
                    palette.button: "#7a2222"
                    onClicked: {
                        // Pass current active profile ID to removal
                        if (LensProfileWrapper.review_entries.length > 0) {
                            var cur = LensProfileWrapper.review_entries[LensProfileWrapper.active_review_index];
                            if (cur && cur.id) {
                                LensProfileWrapper.removeReviewProfile(cur.id);
                            }
                        }
                    }
                }
            }
        }
    }
}
