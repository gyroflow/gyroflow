// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import "CameraCatalog.js" as Catalog

Column {
    id: root
    width: parent.width
    spacing: 6 * dpiScale
    required property var backend
    property bool calibratorMode: false
    property var catalogIndex: null
    property var metadata: ({})
    property string brand: ""
    property string cameraModel: ""
    property string lensModel: ""
    property bool otherBrand: false
    property bool otherCamera: false
    property bool otherLens: false
    property var brands: []
    property var cameras: []
    property var lenses: []
    property var profiles: []
    property var recommendations: []
    property var hidden: ({})
    property string catalogError: ""
    signal selectionEdited(var selection)
    signal profileSelected(var profile)

    function selection() {
        return {camera_brand: brand, camera_model: cameraModel, lens_model: lensModel}
    }
    function changed() {
        refreshChoices()
        selectionEdited(selection())
    }
    function refreshChoices() {
        brands = catalogIndex ? catalogIndex.brands : []
        cameras = catalogIndex && catalogIndex.models[brand] || []
        lenses = catalogIndex ? Catalog.lensLabels(catalogIndex, brand, cameraModel) : []
        brandBox.currentIndex = otherBrand ? brands.length + 1 : Math.max(0, brands.indexOf(brand) + 1)
        cameraBox.currentIndex = otherCamera ? cameras.length + 1 : Math.max(0, cameras.indexOf(cameraModel) + 1)
        lensBox.currentIndex = otherLens ? lenses.length + 1 : Math.max(0, lenses.indexOf(lensModel) + 1)
        recommendations = catalogIndex ? Catalog.compatibleCameras(catalogIndex, brand, cameraModel) : []
        refreshProfiles()
    }
    function refreshProfiles(applyReplacement) {
        var previous = profiles[profileBox.currentIndex]
        var rows = catalogIndex ? Catalog.profilesFor(catalogIndex, brand, cameraModel, lensModel) : []
        profiles = Catalog.visibleProfiles(rows, hidden, includeHidden.checked)
        var keep = previous ? profiles.findIndex(function(p) { return p.id === previous.id }) : -1
        profileBox.currentIndex = keep >= 0 ? keep : (profiles.length ? 0 : -1)
        // Review filters must keep the preview aligned with a replacement row.
        if (applyReplacement && profileBox.currentIndex >= 0 && keep < 0) applyProfile()
    }
    function setMetadata(value) {
        metadata = value || {}
        var p = Catalog.prefill(catalogIndex, metadata)
        brand = p.brand
        cameraModel = p.model
        lensModel = p.lens
        otherBrand = !!brand && (!catalogIndex || catalogIndex.brands.indexOf(brand) < 0)
        otherCamera = !!cameraModel && !Catalog.resolveCamera(catalogIndex, brand, cameraModel)
        otherLens = !!lensModel && (!catalogIndex || Catalog.lensLabels(catalogIndex, brand, cameraModel).indexOf(lensModel) < 0)
        changed()
    }
    function reload() {
        var data = backend.camera_catalog()
        if (!data) return
        try {
            catalogIndex = Catalog.build(JSON.parse(data))
            catalogError = ""
            // A catalogue refresh must preserve an in-progress user selection.
            setMetadata(brand || cameraModel || lensModel ? selection() : metadata)
        } catch (error) {
            catalogError = qsTr("Camera catalogue unavailable. Manual entry and legacy search remain available.")
        }
    }
    function applyProfile() {
        var profile = profiles[profileBox.currentIndex]
        if (profile && profile.id) profileSelected(profile)
    }
    function stepProfile(direction) {
        if (!profiles.length) return
        profileBox.currentIndex = (profileBox.currentIndex + direction + profiles.length) % profiles.length
        applyProfile()
    }
    function toggleHidden() {
        var profile = profiles[profileBox.currentIndex]
        if (!profile) return
        var copy = Object.assign({}, hidden)
        var key = profile.id || profile.checksum
        if (copy[key]) delete copy[key]
        else copy[key] = true
        hidden = copy
        settings.setValue("hiddenLensProfiles", JSON.stringify(hidden))
        refreshProfiles(true)
    }
    Component.onCompleted: {
        try { hidden = JSON.parse(settings.value("hiddenLensProfiles", "{}")) || {} }
        catch (error) { hidden = {} }
        if (calibratorMode) backend.load_profiles(true)
        else reload()
    }
    Connections {
        target: root.backend
        function onAll_profiles_loaded() { root.reload() }
    }
    BasicText { visible: !!root.catalogError; width: parent.width; text: root.catalogError; wrapMode: Text.WordWrap }
    Label {
        text: qsTr("Camera brand")
        ComboBox {
            id: brandBox; width: parent.width
            model: [qsTr("Choose camera brand")].concat(root.brands).concat([qsTr("Other")])
            onActivated: function(index) {
                root.otherBrand = index === root.brands.length + 1
                root.brand = index > 0 && !root.otherBrand ? root.brands[index - 1] : ""
                root.cameraModel = ""; root.lensModel = ""; root.otherCamera = false; root.otherLens = false
                root.changed()
            }
        }
    }
    TextField {
        visible: root.otherBrand; width: parent.width; text: root.brand; placeholderText: qsTr("Enter camera brand")
        onEditingFinished: { root.brand = text.trim(); root.changed() }
    }
    Label {
        text: qsTr("Camera model")
        ComboBox {
            id: cameraBox; width: parent.width
            model: [qsTr("Choose camera model")].concat(root.cameras).concat([qsTr("Other")])
            onActivated: function(index) {
                root.otherCamera = index === root.cameras.length + 1
                root.cameraModel = index > 0 && !root.otherCamera ? root.cameras[index - 1] : ""
                root.lensModel = ""; root.otherLens = false; root.changed()
            }
        }
    }
    TextField {
        visible: root.otherCamera; width: parent.width; text: root.cameraModel; placeholderText: qsTr("Enter camera model")
        onEditingFinished: { root.cameraModel = text.trim(); root.changed() }
    }
    Label {
        text: qsTr("Lens model")
        ComboBox {
            id: lensBox; width: parent.width
            model: [qsTr("Choose lens")].concat(root.lenses).concat([qsTr("Other")])
            onActivated: function(index) {
                root.otherLens = index === root.lenses.length + 1
                root.lensModel = index > 0 && !root.otherLens ? root.lenses[index - 1] : ""
                root.changed()
            }
        }
    }
    TextField {
        visible: root.otherLens; width: parent.width; text: root.lensModel
        placeholderText: qsTr("Enter lens model or built-in lens description")
        onEditingFinished: { root.lensModel = text.trim(); root.changed() }
    }
    BasicText {
        width: parent.width; wrapMode: Text.WordWrap
        property var cameraInfo: Catalog.camera(root.catalogIndex, root.brand, root.cameraModel)
        visible: !!cameraInfo && (!!cameraInfo.crop_factor || (cameraInfo.mounts || []).length > 0)
        text: cameraInfo ? (cameraInfo.mounts || []).join(", ") +
            (cameraInfo.crop_factor ? " · " + qsTr("Crop factor: %1×").arg(cameraInfo.crop_factor) : "") +
            (cameraInfo.sensor_width_mm && cameraInfo.sensor_height_mm ? " · " + cameraInfo.sensor_width_mm + "×" + cameraInfo.sensor_height_mm + " mm" : "") : ""
    }
    Column {
        visible: !root.calibratorMode; width: parent.width; spacing: 6 * dpiScale
        BasicText {
            width: parent.width; wrapMode: Text.WordWrap
            visible: !!root.lensModel && !root.profiles.length
            text: qsTr("No visible calibrated profile for this setup. A Lensfun catalogue entry is not a Gyroflow calibration.")
        }
        ComboBox {
            id: profileBox; width: parent.width; enabled: root.profiles.length > 0
            model: root.profiles.map(Catalog.profileLabel)
            onActivated: root.applyProfile()
        }
        Row {
            spacing: 5 * dpiScale
            Button { text: qsTr("Previous"); enabled: root.profiles.length > 1; onClicked: root.stepProfile(-1) }
            Button { text: qsTr("Load"); enabled: root.profiles.length > 0; onClicked: root.applyProfile() }
            Button { text: qsTr("Next"); enabled: root.profiles.length > 1; onClicked: root.stepProfile(1) }
        }
        Button {
            enabled: root.profiles.length > 0
            property var selected: root.profiles[profileBox.currentIndex]
            text: selected && root.hidden[selected.id || selected.checksum] ? qsTr("Restore profile") : qsTr("Hide profile locally")
            onClicked: root.toggleHidden()
        }
        CheckBox { id: includeHidden; text: qsTr("Show hidden profiles"); checked: false; onCheckedChanged: root.refreshProfiles(true) }
        BasicText {
            visible: root.recommendations.length > 0; width: parent.width; wrapMode: Text.WordWrap
            text: qsTr("Cameras below share a mount and crop factor. Verify the recording mode and lens; this does not guarantee identical calibration.")
        }
        ComboBox {
            visible: root.recommendations.length > 0; width: parent.width
            model: [qsTr("Browse potentially compatible cameras")].concat(root.recommendations.map(function(c) { return c.brand + " " + c.model }))
            onActivated: function(index) {
                if (index <= 0) return
                var selected = root.recommendations[index - 1]
                root.setMetadata({brand: selected.brand, model: selected.model, lens_model: root.lensModel})
            }
        }
    }
}
