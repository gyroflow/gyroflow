// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick

Column {
    id: root;
    width: parent.width;
    spacing: 6 * dpiScale;
    property var profilesMenu;
    property alias selector: selector;
    property var profiles: [];
    property var hiddenProfiles: ({});
    property int hiddenCount: 0;
    property bool reviewing: false;
    property bool ready: false;
    property var currentProfile: profileBox.currentIndex >= 0 ? profiles[profileBox.currentIndex] : null;

    function hiddenKey(profile: var): string { return profile.checksum || profile.id; }
    function refresh(): void {
        if (!ready) return;
        const selected = currentProfile ? currentProfile.id : "";
        const all = JSON.parse(controller.browse_lens_profiles(JSON.stringify(selector.selection), similar.checked));
        const aspect = profilesMenu.currentVideoAspectRatio || 0;
        const swapped = profilesMenu.currentVideoAspectRatioSwapped || 0;
        all.forEach((profile, index) => { profile.order = index; });
        all.sort((a, b) => Number(a.suggested) - Number(b.suggested)
            || Number(!!profilesMenu.favorites[b.checksum]) - Number(!!profilesMenu.favorites[a.checksum])
            || Number(aspect > 0 && b.aspect_ratio === aspect) - Number(aspect > 0 && a.aspect_ratio === aspect)
            || Number(swapped > 0 && b.aspect_ratio === swapped) - Number(swapped > 0 && a.aspect_ratio === swapped)
            || a.order - b.order);
        hiddenCount = all.filter(p => !!hiddenProfiles[hiddenKey(p)]).length;
        profiles = all.filter(p => showHidden.checked || !hiddenProfiles[hiddenKey(p)]);
        profileBox.model = profiles.map(p => (p.suggested ? qsTr("Similar camera") + " · " : "") + p.name);
        profileBox.currentIndex = profiles.findIndex(p => p.id === selected || (!selected && p.checksum === profilesMenu.profileChecksum));
    }
    function syncLoadedProfile(checksum: string): void {
        refresh();
        if (checksum) profileBox.currentIndex = profiles.findIndex(p => p.checksum === checksum);
    }
    function preview(index: int): void {
        if (index < 0 || index >= profiles.length) return;
        profileBox.currentIndex = index;
        profilesMenu.selected_manually = true;
        reviewing = true;
        controller.load_lens_profile(profiles[index].id);
        reviewing = false;
    }
    function setHidden(hidden: bool): void {
        if (!currentProfile) return;
        const key = hiddenKey(currentProfile);
        if (hidden) hiddenProfiles[key] = true;
        else delete hiddenProfiles[key];
        settings.setValue("lensProfileHidden", JSON.stringify(hiddenProfiles));
        const index = profileBox.currentIndex;
        refresh();
        if (hidden && !showHidden.checked && profiles.length) preview(Math.min(index, profiles.length - 1));
    }
    Component.onCompleted: {
        try { hiddenProfiles = JSON.parse(settings.value("lensProfileHidden", "{}")); }
        catch (e) { hiddenProfiles = {}; }
        ready = true;
        refresh();
    }
    Connections {
        target: controller;
        function onAll_profiles_loaded(): void { root.refresh(); }
    }

    Connections {
        target: root.profilesMenu;
        function onCurrentVideoAspectRatioChanged(): void { root.refresh(); }
        function onCurrentVideoAspectRatioSwappedChanged(): void { root.refresh(); }
        function onFavoritesChanged(): void { root.refresh(); }
    }

    CameraSelector {
        id: selector;
        onChanged: root.refresh();
    }
    CheckBox {
        id: similar;
        text: qsTr("Include profiles from similar cameras");
        checked: true;
        onCheckedChanged: root.refresh();
    }
    CatalogueComboBox {
        id: profileBox;
        objectName: "calibrationProfileSelector";
        width: parent.width;
        enabled: root.profiles.length > 0;
        displayText: currentIndex >= 0 ? currentText : qsTr("Choose a calibration (%1)").arg(root.profiles.length);
        onActivated: index => root.preview(index);
    }
    Row {
        spacing: 6 * dpiScale;
        width: parent.width;
        Button {
            text: qsTr("Previous");
            width: (parent.width - parent.spacing) / 2;
            enabled: profileBox.currentIndex > 0;
            onClicked: root.preview(profileBox.currentIndex - 1);
        }
        Button {
            text: qsTr("Next");
            width: (parent.width - parent.spacing) / 2;
            enabled: profileBox.currentIndex < root.profiles.length - 1;
            onClicked: root.preview(profileBox.currentIndex + 1);
        }
    }
    Text {
        width: parent.width;
        color: styleTextColor;
        font.pixelSize: 11 * dpiScale;
        wrapMode: Text.WordWrap;
        text: root.currentProfile ? qsTr("%1 of %2 · %3 × %4 · %5 fps").arg(profileBox.currentIndex + 1).arg(root.profiles.length)
            .arg(root.currentProfile.width).arg(root.currentProfile.height).arg(root.currentProfile.fps.toFixed(2)) : "";
        visible: text.length > 0;
    }
    InfoMessageSmall {
        show: !!root.currentProfile && root.currentProfile.suggested;
        text: qsTr("The mount and sensor crop are similar. Check the lens, recording mode, digital correction and framing before using this calibration.");
    }
    InfoMessageSmall {
        show: selector.cameraBrand.length > 0 && root.profiles.length === 0;
        text: root.hiddenCount ? qsTr("Matching profiles are hidden. Enable Show hidden profiles to restore them.") :
            qsTr("No Gyroflow calibration matches this selection. Choose another lens or create a new calibration.");
    }
    Row {
        spacing: 6 * dpiScale;
        width: parent.width;
        visible: root.currentProfile !== null && root.currentProfile !== undefined;
        Button {
            width: (parent.width - parent.spacing) / 2;
            text: root.currentProfile && root.hiddenProfiles[root.hiddenKey(root.currentProfile)] ? qsTr("Restore profile") : qsTr("Hide profile");
            onClicked: root.setHidden(!root.hiddenProfiles[root.hiddenKey(root.currentProfile)]);
            tooltip: qsTr("Hide this calibration from the review list on this device. Use the profile rating to report an incorrect calibration.");
        }
        Button {
            width: (parent.width - parent.spacing) / 2;
            text: root.currentProfile && profilesMenu.favorites[root.currentProfile.checksum] ? qsTr("Unfavorite") : qsTr("Favorite");
            onClicked: {
                const key = root.currentProfile.checksum;
                if (!key) return;
                if (profilesMenu.favorites[key]) delete profilesMenu.favorites[key];
                else profilesMenu.favorites[key] = 1;
                profilesMenu.updateFavorites();
                profilesMenu.loadFavorites();
            }
        }
    }
    CheckBox {
        id: showHidden;
        objectName: "showHiddenProfiles";
        text: qsTr("Show hidden profiles (%1)").arg(root.hiddenCount);
        visible: root.hiddenCount > 0 || checked;
        onCheckedChanged: root.refresh();
    }
}
