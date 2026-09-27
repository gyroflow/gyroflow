// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick

Column {
    id: root;
    width: parent.width;
    spacing: 5 * dpiScale;

    property var cameras: [];
    property var compatibleCameras: [];
    property bool allowManualEntry: false;
    signal selectionChanged(string brand, string model, string lens);
    signal manualEntryRequested();

    function unique(values) {
        let out = [];
        for (const value of values) {
            if (value && out.indexOf(value) < 0) out.push(value);
        }
        out.sort();
        return out;
    }

    function selectedBrand() {
        return brandBox.currentIndex > 0 ? brandBox.currentText : "";
    }

    function selectedModel() {
        return modelBox.currentIndex > 0 ? modelBox.currentText : "";
    }

    function selectedLens() {
        return lensBox.currentIndex > 0 ? lensBox.currentText : "";
    }

    function loadCatalogue() {
        let parsed = { "cameras": [] };
        try {
            parsed = JSON.parse(controller.camera_catalogue_json());
        } catch (e) {
            console.warn("Unable to load camera catalogue", e);
        }
        cameras = parsed.cameras || [];
        brandBox.model = [qsTr("All camera brands")].concat(unique(cameras.map(c => c.brand)));
        brandBox.currentIndex = 0;
        refreshModels();
    }

    function refreshModels() {
        const brand = selectedBrand();
        const models = brand ? cameras.filter(c => c.brand === brand).map(c => c.model) : [];
        modelBox.model = [qsTr("All camera models")].concat(unique(models));
        modelBox.currentIndex = 0;
        refreshLenses();
    }

    function refreshLenses() {
        const brand = selectedBrand();
        const model = selectedModel();
        let lenses = [];
        if (brand && model) {
            const camera = cameras.find(c => c.brand === brand && c.model === model);
            lenses = camera ? (camera.lenses || []) : [];
        }
        lensBox.model = [qsTr("All lenses")].concat(unique(lenses));
        lensBox.currentIndex = 0;
        refreshCompatibleCameras();
    }

    function refreshCompatibleCameras() {
        const brand = selectedBrand();
        const model = selectedModel();
        const source = cameras.find(c => c.brand === brand && c.model === model);
        if (!source || !source.crop_factor || source.crop_factor <= 0) {
            compatibleCameras = [];
            return;
        }
        compatibleCameras = cameras.filter(c =>
            c.brand === source.brand
            && c.model !== source.model
            && c.crop_factor > 0
            && Math.abs(c.crop_factor - source.crop_factor) / source.crop_factor <= 0.01
        );
    }

    function emitSelection() {
        selectionChanged(selectedBrand(), selectedModel(), selectedLens());
    }

    function findTextIndex(model, value) {
        if (!value) return 0;
        const wanted = value.toLowerCase();
        for (let i = 1; i < model.length; ++i) {
            if (("" + model[i]).toLowerCase() === wanted) return i;
        }
        return 0;
    }

    function setSelection(brand, model, lens) {
        brandBox.currentIndex = findTextIndex(brandBox.model, brand);
        refreshModels();
        modelBox.currentIndex = findTextIndex(modelBox.model, model);
        refreshLenses();
        lensBox.currentIndex = findTextIndex(lensBox.model, lens);
    }

    Component.onCompleted: loadCatalogue();
    Connections {
        target: controller;
        function onAll_profiles_loaded(): void { root.loadCatalogue(); }
    }

    ComboBox {
        id: brandBox;
        objectName: "cameraBrandSelector";
        width: parent.width;
        model: [qsTr("All camera brands")];
        onActivated: {
            root.refreshModels();
            root.emitSelection();
        }
    }

    ComboBox {
        id: modelBox;
        objectName: "cameraModelSelector";
        width: parent.width;
        enabled: brandBox.currentIndex > 0;
        model: [qsTr("All camera models")];
        onActivated: {
            root.refreshLenses();
            root.emitSelection();
        }
    }

    ComboBox {
        id: lensBox;
        objectName: "lensModelSelector";
        width: parent.width;
        enabled: modelBox.currentIndex > 0;
        model: [qsTr("All lenses")];
        onActivated: root.emitSelection();
    }

    Button {
        objectName: "cameraManualEntryButton";
        width: parent.width;
        visible: root.allowManualEntry;
        text: qsTr("Other / manual setup");
        onClicked: root.manualEntryRequested();
    }

    BasicText {
        objectName: "compatibleCameraHint";
        width: parent.width;
        visible: root.compatibleCameras.length > 0;
        wrapMode: Text.WordWrap;
        font.pixelSize: 10 * dpiScale;
        text: qsTr("Compatible cameras: %1").arg(root.compatibleCameras.map(c => c.brand + " " + c.model).join(", "));
    }
}
