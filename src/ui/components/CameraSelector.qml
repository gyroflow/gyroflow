// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick

Column {
    id: root;
    spacing: 6 * dpiScale;
    width: parent.width;

    property bool allowOther: false;
    property string cameraBrand: "";
    property string cameraModel: "";
    property string lensModel: "";
    property var catalogue: ({});
    property bool manualBrand: false;
    property bool manualModel: false;
    property bool manualLens: false;
    property bool ready: false;
    property bool updating: false;
    readonly property var selection: ({camera_brand: cameraBrand, camera_model: cameraModel, lens_model: lensModel});
    readonly property bool zoomLens: (catalogue.lenses || []).some(l => l.name === lensModel && l.zoom) || /\d\s*[-–—]\s*\d/.test(lensModel);
    signal changed();
    signal edited();

    function choices(items: var, current: string, placeholder: string): var {
        let result = [placeholder].concat(items || []);
        if (current && result.indexOf(current) < 0) result.push(current);
        if (allowOther) result.push(qsTr("Other…"));
        return result;
    }
    function setSelection(brand: string, model: string, lens: string): void {
        cameraBrand = brand || "";
        cameraModel = model || "";
        lensModel = lens || "";
        manualBrand = manualModel = manualLens = false;
        refresh();
    }
    function refresh(): void {
        if (!ready || updating) return;
        updating = true;
        catalogue = JSON.parse(controller.camera_selector_options(JSON.stringify(selection)));
        const canonical = catalogue.selection || selection;
        cameraBrand = canonical.camera_brand || "";
        cameraModel = canonical.camera_model || "";
        lensModel = canonical.lens_model || "";
        brandBox.model = choices(catalogue.brands, cameraBrand, qsTr("Camera brand"));
        modelBox.model = choices(catalogue.models, cameraModel, allowOther ? qsTr("Camera model") : qsTr("All camera models"));
        lensBox.model = choices((catalogue.lenses || []).map(l => l.name), lensModel, allowOther ? qsTr("Lens model or built-in mode") : qsTr("All lenses"));
        modelBox.searchAliases = catalogue.model_aliases || {};
        let lensAliases = {};
        for (const lens of catalogue.lenses || []) lensAliases[lens.name] = lens.aliases || [];
        lensBox.searchAliases = lensAliases;
        brandBox.currentIndex = manualBrand ? brandBox.count - 1 : Math.max(0, brandBox.model.indexOf(cameraBrand));
        modelBox.currentIndex = manualModel ? modelBox.count - 1 : Math.max(0, modelBox.model.indexOf(cameraModel));
        lensBox.currentIndex = manualLens ? lensBox.count - 1 : Math.max(0, lensBox.model.indexOf(lensModel));
        brandInput.text = cameraBrand;
        modelInput.text = cameraModel;
        lensInput.text = lensModel;
        updating = false;
        changed();
    }

    Component.onCompleted: { ready = true; refresh(); }
    Connections {
        target: controller;
        function onAll_profiles_loaded(): void { root.refresh(); }
    }

    CatalogueComboBox {
        id: brandBox;
        includeOther: root.allowOther;
        objectName: "cameraBrandSelector";
        width: parent.width;
        onActivated: index => {
            root.manualBrand = root.allowOther && index === count - 1;
            root.manualModel = root.manualLens = false;
            root.cameraBrand = root.manualBrand ? query.trim() : index > 0 ? currentText : "";
            root.cameraModel = root.lensModel = "";
            root.refresh();
            root.edited();
            if (root.manualBrand) brandInput.forceActiveFocus();
        }
    }
    TextField {
        id: brandInput;
        objectName: "cameraBrandInput";
        width: parent.width;
        visible: root.manualBrand;
        placeholderText: qsTr("Camera manufacturer");
        onEditingFinished: {
            if (root.updating) return;
            root.cameraBrand = text.trim();
            root.cameraModel = root.lensModel = "";
            root.refresh();
            root.edited();
        }
    }
    CatalogueComboBox {
        id: modelBox;
        includeOther: root.allowOther;
        objectName: "cameraModelSelector";
        width: parent.width;
        enabled: root.cameraBrand.length > 0;
        onActivated: index => {
            root.manualModel = root.allowOther && index === count - 1;
            root.manualLens = false;
            root.cameraModel = root.manualModel ? query.trim() : index > 0 ? currentText : "";
            root.lensModel = "";
            root.refresh();
            root.edited();
            if (root.manualModel) modelInput.forceActiveFocus();
        }
    }
    TextField {
        id: modelInput;
        objectName: "cameraModelInput";
        width: parent.width;
        visible: root.manualModel;
        placeholderText: qsTr("Exact camera model");
        onEditingFinished: {
            if (root.updating) return;
            root.cameraModel = text.trim();
            root.lensModel = "";
            root.refresh();
            root.edited();
        }
    }
    CatalogueComboBox {
        id: lensBox;
        includeOther: root.allowOther;
        objectName: "lensModelSelector";
        width: parent.width;
        enabled: root.cameraModel.length > 0;
        onActivated: index => {
            root.manualLens = root.allowOther && index === count - 1;
            root.lensModel = root.manualLens ? query.trim() : index > 0 ? currentText : "";
            root.refresh();
            root.edited();
            if (root.manualLens) lensInput.forceActiveFocus();
        }
    }
    TextField {
        id: lensInput;
        objectName: "lensModelInput";
        width: parent.width;
        visible: root.manualLens;
        placeholderText: qsTr("Lens manufacturer and model, or built-in mode");
        onEditingFinished: {
            if (root.updating) return;
            root.lensModel = text.trim();
            root.refresh();
            root.edited();
        }
    }
    Text {
        width: parent.width;
        color: styleTextColor;
        font.pixelSize: 11 * dpiScale;
        wrapMode: Text.WordWrap;
        textFormat: Text.PlainText;
        visible: text.length > 0;
        text: {
            const c = root.catalogue.camera;
            if (!c) return "";
            let parts = [];
            if (c.mounts && c.mounts.length) parts.push(qsTr("Mount: %1").arg(c.mounts.join(", ")));
            if (c.crop_factor > 0) {
                parts.push(qsTr("Sensor crop: %1×").arg(c.crop_factor.toFixed(2)));
                if (c.sensor_width_mm && c.sensor_height_mm)
                    parts.push(qsTr("Sensor: %1 × %2 mm").arg(c.sensor_width_mm.toFixed(1)).arg(c.sensor_height_mm.toFixed(1)));
                else parts.push(qsTr("Sensor diagonal: %1 mm").arg((Math.sqrt(36*36 + 24*24) / c.crop_factor).toFixed(1)));
            }
            return parts.join(" · ");
        }
    }
    Text {
        width: parent.width;
        color: styleTextColor;
        font.pixelSize: 11 * dpiScale;
        wrapMode: Text.WordWrap;
        textFormat: Text.PlainText;
        visible: text.length > 0;
        text: {
            const l = (root.catalogue.lenses || []).find(l => l.name === root.lensModel);
            if (!l) return "";
            let parts = [];
            if (l.adapted) parts.push(qsTr("This lens mount requires an adapter."));
            if (!l.has_profiles) parts.push(qsTr("Known lens; no Gyroflow calibration is listed for this camera."));
            return parts.join(" ");
        }
    }
    Text {
        width: parent.width;
        color: styleTextColor;
        font.pixelSize: 10 * dpiScale;
        wrapMode: Text.WordWrap;
        textFormat: Text.StyledText;
        linkColor: styleAccentColor;
        text: qsTr("Camera data: %1 and %2.")
            .arg('<a href="https://github.com/gyroflow/lens_profiles">Gyroflow</a>')
            .arg('<a href="https://lensfun.github.io/">Lensfun</a> (<a href="https://creativecommons.org/licenses/by-sa/3.0/">CC BY-SA 3.0</a>)');
        onLinkActivated: link => Qt.openUrlExternally(link);
    }
}
