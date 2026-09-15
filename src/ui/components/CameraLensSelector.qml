// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2026 Adrian <adrian.eddy at gmail>

import QtQuick

Column {
    id: root;
    width: parent.width;
    spacing: 8 * dpiScale;

    property string brand: "";
    property string model: "";
    property string lens: "";
    property real cropFactor: 0;
    property string mountName: "";
    property string sensorName: "";
    property bool allowOther: true;
    property bool suppress: false;
    property var compatible: [];

    readonly property string otherLabel: qsTr("Other");
    readonly property bool usingOther: brandBox.currentText === root.otherLabel || modelBox.currentText === root.otherLabel || lensBox.currentText === root.otherLabel;
    readonly property bool identityComplete: root.isFilledName(root.brand) && root.isFilledName(root.model);

    signal selectionChanged();
    signal userSelectionChanged();

    function listToArray(list) {
        const out = [];
        if (!list) return out;
        for (let i = 0; i < list.length; ++i) out.push(list[i]);
        return out;
    }
    function withOther(list) {
        const out = root.listToArray(list);
        if (root.allowOther) out.push(root.otherLabel);
        return out;
    }
    function setCombo(box: var, value: string): void {
        if (!value) {
            box.currentIndex = -1;
            return;
        }
        const idx = box.model.indexOf(value);
        if (idx >= 0) {
            box.currentIndex = idx;
            return;
        }
        if (root.allowOther) {
            const o = box.model.indexOf(root.otherLabel);
            box.currentIndex = o >= 0? o : -1;
        }
    }
    function currentBrand(): string {
        return brandBox.currentText === root.otherLabel? otherBrand.text : (brandBox.currentText || "");
    }
    function currentModel(): string {
        return modelBox.currentText === root.otherLabel? otherModel.text : (modelBox.currentText || "");
    }
    function currentLens(): string {
        return lensBox.currentText === root.otherLabel? otherLens.text : (lensBox.currentText || "");
    }
    function isFilledName(value: string): bool {
        const t = (value || "").trim();
        return t.length > 0 && t !== root.otherLabel && t !== "---" && t !== "-" && t.toLowerCase() !== "unknown" && t.toLowerCase() !== "n/a";
    }
    function emitTyped(): void {
        if (root.suppress) return;
        const obj = controller.resolve_camera_selection(root.currentBrand(), root.currentModel(), root.currentLens());
        root.applyResolved(obj);
        root.selectionChanged();
    }
    function applyResolved(obj: var): void {
        root.brand = obj.brand || "";
        root.model = obj.model || "";
        root.lens  = obj.lens  || "";
        root.cropFactor = +obj.crop_factor || 0;
        root.mountName  = obj.mount || "";
        root.sensorName = obj.sensor_size || "";
        root.compatible = root.listToArray(controller.get_compatible_cameras(root.brand, root.model));
        if (brandBox.currentText === root.otherLabel) root.brand = otherBrand.text;
        if (modelBox.currentText === root.otherLabel) root.model = otherModel.text;
        if (lensBox.currentText  === root.otherLabel) root.lens  = otherLens.text;
    }
    function emitSelection(): void {
        const obj = controller.resolve_camera_selection(root.currentBrand(), root.currentModel(), root.currentLens());
        root.applyResolved(obj);
        root.selectionChanged();
        if (!root.suppress) root.userSelectionChanged();
    }
    function refreshBrands(): void {
        brandBox.model = root.withOther(controller.get_camera_brands());
    }
    function prefill(brand: string, model: string, lens: string): void {
        root.suppress = true;
        root.refreshBrands();
        const obj = controller.resolve_camera_selection(brand || "", model || "", lens || "");
        const resolvedBrand = obj.brand || brand || "";
        const resolvedModel = obj.model || model || "";
        const resolvedLens  = obj.lens  || lens  || "";
        root.setCombo(brandBox, resolvedBrand);
        modelBox.model = root.withOther(controller.get_camera_models(resolvedBrand));
        root.setCombo(modelBox, resolvedModel);
        lensBox.model = root.withOther(controller.get_camera_lenses(resolvedBrand, resolvedModel));
        root.setCombo(lensBox, resolvedLens);
        if (brandBox.currentText === root.otherLabel) otherBrand.text = brand || "";
        if (modelBox.currentText === root.otherLabel) otherModel.text = model || "";
        if (lensBox.currentText  === root.otherLabel) otherLens.text  = lens  || "";
        root.applyResolved(obj);
        root.suppress = false;
        root.selectionChanged();
    }

    Connections {
        target: controller;
        function onCamera_database_loaded(): void {
            const b = root.brand, m = root.model, l = root.lens;
            if (b) root.prefill(b, m, l);
            else root.refreshBrands();
        }
    }
    Component.onCompleted: root.refreshBrands();

    Label {
        position: Label.LeftPosition;
        text: qsTr("Camera brand");
        ComboBox {
            id: brandBox;
            width: parent.width;
            font.pixelSize: 12 * dpiScale;
            model: [];
            onCurrentIndexChanged: {
                if (root.suppress) return;
                const brand = currentText === root.otherLabel? "" : currentText;
                root.suppress = true;
                modelBox.model = root.withOther(controller.get_camera_models(brand));
                modelBox.currentIndex = -1;
                lensBox.model = root.withOther(controller.get_camera_lenses(brand, ""));
                lensBox.currentIndex = -1;
                root.suppress = false;
                root.emitSelection();
            }
        }
    }
    TextField {
        id: otherBrand;
        visible: brandBox.currentText === root.otherLabel;
        width: parent.width;
        placeholderText: qsTr("Type the camera brand");
        onTextChanged: root.emitTyped();
        onEditingFinished: root.emitSelection();
    }

    Label {
        position: Label.LeftPosition;
        text: qsTr("Camera model");
        ComboBox {
            id: modelBox;
            width: parent.width;
            font.pixelSize: 12 * dpiScale;
            model: [];
            onCurrentIndexChanged: {
                if (root.suppress) return;
                root.suppress = true;
                lensBox.model = root.withOther(controller.get_camera_lenses(root.currentBrand(), currentText === root.otherLabel? "" : currentText));
                lensBox.currentIndex = -1;
                root.suppress = false;
                root.emitSelection();
            }
        }
    }
    TextField {
        id: otherModel;
        visible: modelBox.currentText === root.otherLabel;
        width: parent.width;
        placeholderText: qsTr("Type the camera model");
        onTextChanged: root.emitTyped();
        onEditingFinished: root.emitSelection();
    }

    Label {
        position: Label.LeftPosition;
        text: qsTr("Lens");
        ComboBox {
            id: lensBox;
            width: parent.width;
            font.pixelSize: 12 * dpiScale;
            model: [];
            onCurrentIndexChanged: {
                if (root.suppress) return;
                root.emitSelection();
            }
        }
    }
    TextField {
        id: otherLens;
        visible: lensBox.currentText === root.otherLabel;
        width: parent.width;
        placeholderText: qsTr("Type the lens model");
        onTextChanged: root.emitTyped();
        onEditingFinished: root.emitSelection();
    }

    BasicText {
        visible: root.usingOther;
        width: parent.width;
        wrapMode: Text.WordWrap;
        font.pixelSize: 11 * dpiScale;
        text: qsTr("Other lets you type a name that is not in the catalog. Export still needs a real brand and model, not \"Other\".");
    }

    BasicText {
        visible: root.mountName.length > 0 || root.sensorName.length > 0 || root.cropFactor > 0;
        width: parent.width;
        wrapMode: Text.WordWrap;
        font.pixelSize: 11 * dpiScale;
        text: {
            const bits = [];
            if (root.mountName) bits.push(root.mountName);
            if (root.sensorName) bits.push(root.sensorName);
            if (root.cropFactor > 0) bits.push(qsTr("%1x crop").arg(root.cropFactor.toFixed(2)));
            return bits.join(" · ");
        }
    }
    BasicText {
        visible: root.compatible.length > 0;
        width: parent.width;
        wrapMode: Text.WordWrap;
        font.pixelSize: 11 * dpiScale;
        text: qsTr("Compatible cameras: %1").arg(
            root.compatible.map(c => (c.brand + " " + c.model).trim()).join(", ")
        );
    }
}
