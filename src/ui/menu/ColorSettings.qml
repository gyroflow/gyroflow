// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import "../components/"

MenuItem {
    id: root;
    text: qsTr("Color settings");
    iconName: "color";
    objectName: "color";
    innerItem.enabled: window.videoArea.vid.loaded;

    // Keep project, preset and queue color data in the existing export model.
    required property QtObject exportOptions;
    property bool syncing: true;
    property var recentLuts: [];

    function lutKey(url: string): string {
        return filesystem.url_to_path(url);
    }
    function loadRecentLuts(): void {
        try {
            const urls = JSON.parse(settings.value("recentColorLuts", "[]"));
            if (Array.isArray(urls)) {
                recentLuts = urls.filter((url, index) => typeof url === "string"
                    && url.startsWith("file:") && /\.cube$/i.test(url)
                    && urls.findIndex(x => typeof x === "string" && lutKey(x) === lutKey(url)) === index).slice(0, 8);
            }
        } catch (e) { recentLuts = []; }
    }
    function updateRecentSelection(): void {
        recentSelector.currentIndex = recentLuts.findIndex(url => lutKey(url) === lutKey(exportOptions.lutUrl));
    }
    function rememberLut(): void {
        const url = exportOptions.lutUrl;
        if (url && !exportOptions.lutPreviewError) {
            const urls = [url].concat(recentLuts.filter(x => lutKey(x) !== lutKey(url))).slice(0, 8);
            if (JSON.stringify(urls) !== JSON.stringify(recentLuts)) {
                recentLuts = urls;
                settings.setValue("recentColorLuts", JSON.stringify(urls));
            }
        }
        updateRecentSelection();
    }
    function chooseLut(url: string): void {
        // Reselecting the current file also refreshes its cached preview.
        if (lutKey(exportOptions.lutUrl) === lutKey(url)) exportOptions.lutUrl = "";
        exportOptions.lutUrl = url;
    }
    onRecentLutsChanged: updateRecentSelection();

    function syncSliders(): void {
        syncing = true;
        brightnessSlider.value = exportOptions.brightness;
        contrastSlider.value = exportOptions.contrast;
        syncing = false;
    }
    Component.onCompleted: {
        loadRecentLuts();
        syncSliders();
        rememberLut();
    }
    Connections {
        target: root.exportOptions;
        function onLutUrlChanged(): void {
            // Wait for LUT validation before recording a successful selection.
            Qt.callLater(root.rememberLut);
        }
        function onBrightnessChanged(): void { root.syncSliders(); }
        function onContrastChanged(): void { root.syncSliders(); }
    }

    FileDialog {
        id: lutDialog;
        title: qsTr("Choose a LUT");
        nameFilters: [qsTr("3D LUT files") + " (*.cube *.CUBE)"];
        type: "export-lut";
        onAccepted: root.chooseLut(selectedFile.toString());
    }
    Label {
        text: qsTr("LUT");
        Row {
            spacing: 6 * dpiScale;
            Button {
                text: qsTr("Choose LUT…");
                onClicked: lutDialog.open2();
            }
            Button {
                text: qsTr("Clear");
                visible: !!root.exportOptions.lutUrl;
                onClicked: root.exportOptions.lutUrl = "";
            }
        }
    }
    Label {
        text: qsTr("Recent LUTs");
        ComboBox {
            id: recentSelector;
            width: parent.width;
            enabled: root.recentLuts.length > 0;
            model: root.recentLuts.map(url => {
                const filename = filesystem.get_filename(url);
                const duplicate = root.recentLuts.some(x => x !== url && filesystem.get_filename(x) === filename);
                return duplicate ? filename + " — " + filesystem.url_to_path(filesystem.get_folder(url)) : filename;
            });
            displayText: currentIndex >= 0 ? currentText : qsTr("Choose a recent LUT…");
            tooltip: currentIndex >= 0 ? filesystem.url_to_path(root.recentLuts[currentIndex]) : "";
            onActivated: (index) => root.chooseLut(root.recentLuts[index]);
        }
    }
    InfoMessageSmall {
        show: !!root.exportOptions.lutPreviewError;
        type: InfoMessage.Error;
        text: root.exportOptions.lutPreviewError;
    }
    Rectangle {
        width: parent.width;
        height: lutStatus.height + 24 * dpiScale;
        visible: !!root.exportOptions.lutUrl && !root.exportOptions.lutPreviewError;
        radius: 6 * dpiScale;
        color: Qt.rgba(0.22, 0.70, 0.44, 0.10);
        border.color: Qt.rgba(0.22, 0.70, 0.44, 0.35);
        Column {
            id: lutStatus;
            x: 12 * dpiScale;
            y: 12 * dpiScale;
            width: parent.width - 24 * dpiScale;
            spacing: 5 * dpiScale;
            BasicText {
                leftPadding: 0;
                text: qsTr("✓ LUT applied to export");
                font.bold: true;
                color: "#54bf85";
            }
            BasicText {
                width: parent.width;
                leftPadding: 0;
                text: filesystem.get_filename(root.exportOptions.lutUrl);
                wrapMode: Text.Wrap;
            }
            BasicText {
                width: parent.width;
                leftPadding: 0;
                text: root.exportOptions.previewColors ? qsTr("Shown in the preview.") : qsTr("Preview colors are switched off.");
                font.pixelSize: 11 * dpiScale;
                opacity: 0.7;
                wrapMode: Text.WordWrap;
            }
        }
    }

    Label {
        text: qsTr("Brightness");
        SliderWithField {
            id: brightnessSlider;
            onValueChanged: if (!root.syncing) root.exportOptions.brightness = value;
            doubleClickResetEnabled: true;
            width: parent.width;
            from: -50; to: 50; field.from: -50; field.to: 50; defaultValue: 0; precision: 0; unit: "%";
        }
    }
    Label {
        text: qsTr("Contrast");
        SliderWithField {
            id: contrastSlider;
            onValueChanged: if (!root.syncing) root.exportOptions.contrast = value;
            doubleClickResetEnabled: true;
            width: parent.width;
            from: -50; to: 50; field.from: -50; field.to: 50; defaultValue: 0; precision: 0; unit: "%";
        }
    }
    Row {
        spacing: 8 * dpiScale;
        CheckBox {
            text: qsTr("Preview colors");
            checked: root.exportOptions.previewColors;
            onToggled: root.exportOptions.previewColors = checked;
        }
        Button {
            text: qsTr("Reset adjustments");
            enabled: root.exportOptions.brightness !== 0 || root.exportOptions.contrast !== 0;
            onClicked: { root.exportOptions.brightness = 0; root.exportOptions.contrast = 0; }
        }
    }
    BasicText {
        width: parent.width;
        wrapMode: Text.WordWrap;
        font.pixelSize: 11 * dpiScale;
        opacity: 0.7;
        text: qsTr("Brightness and contrast are applied after the LUT and included in export. Double-click a slider to reset it.");
    }

}
