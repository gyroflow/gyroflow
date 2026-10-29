// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

import QtQuick

// Shows the tracked points of "Analyze image optically" (Motion data -> Optical correction -> Show/edit tracked points)
// over the stabilized preview, and takes them out. The core draws the points into the frame as it renders it
// (`StabilizationManager::draw_optical_points`, on the GPU's drawing canvas), placed the way the render places the
// picture, so they sit on what they were tracked on; this is a child of the video item that keeps the selection, finds
// the points under the mouse, and shows the box dragged. With the stabilization off nothing renders the frame, and this
// is hidden. What's selected is tracks, not points: the selection follows them from frame to frame, drawn through the
// frames around the one shown. Taking them out measures the analysis again without them in the core
// (`StabilizationManager::remove_optical_tracks`), and the preview stabilizes again with that; the points are gone
// right away. Ctrl+Z brings them back
Item {
    id: root;
    property bool active: false;
    property real timestampUs: 0;
    // Whether the preview shows the stabilized picture: the points are drawn only there
    property bool stabilized: true;
    // How many frames before and after the one shown the selected tracks are drawn through
    property int trailFrames: 30;
    // Trails for at most this many tracks: a box over the whole clip can select thousands
    property int maxTrails: 400;

    // The frame needs drawing again: what the core draws of the points changed
    signal redraw();

    // From the core, at `pointsAt` (see `framePoints`): [x, y, state, track, ...], state as `PointState`; not the tracks
    // taken out
    property var points: [];
    property real pointsAt: -1;
    // The selected tracks: { track: true }
    property var selection: ({});
    property int selectionCount: 0;
    // What was taken out, the last one last, for Ctrl+Z: the tracks of each removal
    property var history: [];
    // A press held, and the box dragged from it
    property var pending: null;
    property var band: null;
    // The search for the tracks in the box let go of (`find_optical_tracks_in_rect`, 0 for none), and whether what it
    // finds adds to the selection. The box stays until then: over every frame of a long clip it takes a moment
    property real search: 0;
    property bool searchAdds: false;
    // The analysis whose tracks the selection and the history are of: another one numbers its tracks anew
    property var tracksKey: null;

    // As the core draws the selected points and their trails (`Color::Sky`)
    readonly property color selectedColor: "#3fb6ff";

    visible: active && stabilized;

    // The points of the frame shown, asked for once the mouse needs them there rather than with every frame
    function framePoints(): var {
        if (pointsAt !== timestampUs) {
            try { points = JSON.parse(controller.optical_tracked_points(timestampUs, stabilized)); } catch (e) { points = []; }
            pointsAt = timestampUs;
        }
        return points;
    }
    // What the core draws: the points, the selected ones, and the trails of the first `maxTrails` of those
    function updateOverlay(): void {
        controller.set_optical_points_overlay(active && stabilized, JSON.stringify(active? selectedIds() : []), trailFrames, maxTrails);
        root.redraw();
    }
    // A new analysis: what's selected and what was taken out were of the tracks before
    function checkTracks(): void {
        let key = null;
        try { key = JSON.parse(controller.optical_correction_info()).tracks_key; } catch (e) { }
        if (key !== tracksKey) {
            tracksKey = key;
            history = [];
            setSelection([], false);
        }
    }
    onActiveChanged: {
        cancelSearch();
        pending = null; band = null;
        pointsAt = -1;
        setSelection([], false);
        checkTracks();
        // Not focused until clicked: it turns on by itself after an analysis, and takes over Ctrl+A
    }
    onStabilizedChanged: {
        cancelSearch();
        pending = null; band = null;
        pointsAt = -1;
        updateOverlay();
    }
    Connections {
        target: controller;
        function onOptical_tracks_changed(): void { if (root.active) { root.checkTracks(); root.pointsAt = -1; root.redraw(); } }
        // Stabilized again (a removal, or any other change): the points are elsewhere in the picture now
        function onCompute_progress(id: real, progress: real): void { if (progress >= 1.0) root.pointsAt = -1; }
        function onChart_data_changed(): void { root.pointsAt = -1; }
        function onOptical_tracks_found(search: real, ids: string): void {
            if (search !== root.search) return;
            root.search = 0;
            root.band = null;
            ma.cursorShape = Qt.CrossCursor;
            let found = [];
            try { found = JSON.parse(ids); } catch (e) { }
            root.setSelection(found, root.searchAdds);
        }
    }
    function cancelSearch(): void {
        if (search === 0) return;
        controller.cancel_optical_track_search();
        search = 0;
        band = null;
        ma.cursorShape = Qt.CrossCursor;
    }

    // Units of this item per pixel on the screen: the video item is scaled to fit
    function unit(): real {
        const a = root.mapToItem(null, 0, 0), b = root.mapToItem(null, 100, 0);
        return 100 / Math.max(0.001, Math.hypot(b.x - a.x, b.y - a.y));
    }
    function selectedIds(): var { return Object.keys(selection).map(Number); }
    function setSelection(ids: var, add: bool): void {
        const s = add? Object.assign({}, selection) : {};
        for (const id of ids) s[id] = true;
        selection = s;
        selectionCount = Object.keys(s).length;
        updateOverlay();
    }
    function toggle(id: int): void {
        const s = Object.assign({}, selection);
        if (s[id]) delete s[id]; else s[id] = true;
        selection = s;
        selectionCount = Object.keys(s).length;
        updateOverlay();
    }
    // The track of the point closest to (x, y), within a few pixels on the screen; -1 for none
    function pointAt(x: real, y: real): int {
        const pts = framePoints();
        let best = -1, bestDistance = 7 * dpiScale * unit();
        for (let i = 0; i + 3 < pts.length; i += 4) {
            const d = Math.hypot(pts[i] * width - x, pts[i + 1] * height - y);
            if (d <= bestDistance) { bestDistance = d; best = pts[i + 3]; }
        }
        return best;
    }
    // The tracks of the points in this frame, of the states `states` (all without)
    function tracksHere(states: var): var {
        const pts = framePoints();
        const ids = [];
        for (let i = 0; i + 3 < pts.length; i += 4) {
            if (!states || states.includes(pts[i + 2])) ids.push(pts[i + 3]);
        }
        return ids;
    }
    // Takes the selected tracks out. The core says which of them weren't already, which is what Ctrl+Z puts back
    function removeSelected(): void {
        let removed = [];
        try { removed = JSON.parse(controller.remove_optical_tracks(JSON.stringify(selectedIds()), true)); } catch (e) { }
        if (removed.length > 0) history = history.concat([removed]);
        setSelection([], false);
    }
    function undo(): void {
        // A removal that couldn't be measured is undone already (the core put its tracks back): nothing of it to undo
        while (history.length > 0) {
            const last = history[history.length - 1];
            history = history.slice(0, -1);
            let back = [];
            try { back = JSON.parse(controller.remove_optical_tracks(JSON.stringify(last), false)); } catch (e) { }
            if (back.length > 0) {
                // Selected, to show what came back
                setSelection(back, false);
                return;
            }
        }
    }

    // The box dragged: orange over every frame (Ctrl), blue over this one
    Rectangle {
        readonly property var b: root.band;
        visible: !!b;
        x: b? Math.min(b.x0, b.x1) : 0;
        y: b? Math.min(b.y0, b.y1) : 0;
        width: b? Math.abs(b.x1 - b.x0) : 0;
        height: b? Math.abs(b.y1 - b.y0) : 0;
        color: b && b.allFrames? "#1fffa032" : "#1f3fb6ff";
        border.color: b && b.allFrames? "#ffa032" : root.selectedColor;
        border.width: b? dpiScale * root.unit() : 1;
    }

    MouseArea {
        id: ma;
        anchors.fill: parent;
        hoverEnabled: true;
        acceptedButtons: Qt.LeftButton | Qt.RightButton;
        cursorShape: Qt.CrossCursor;

        onPressed: (mouse) => {
            root.forceActiveFocus();
            root.cancelSearch();
            if (mouse.button === Qt.RightButton) {
                // On a point not selected: that one alone
                const id = root.pointAt(mouse.x, mouse.y);
                if (id >= 0 && !root.selection[id]) root.setSelection([id], false);
                const pos = root.mapToItem(window.videoArea, mouse.x, mouse.y);
                pointMenu.popup(window.videoArea, pos.x, pos.y);
                return;
            }
            root.pending = { x: mouse.x, y: mouse.y };
        }
        onPositionChanged: (mouse) => {
            const p = root.pending;
            if (!p || !pressed) {
                cursorShape = root.search? Qt.BusyCursor : root.pointAt(mouse.x, mouse.y) >= 0? Qt.PointingHandCursor : Qt.CrossCursor;
                return;
            }
            if (root.band || Math.hypot(mouse.x - p.x, mouse.y - p.y) > 4 * dpiScale * root.unit()) {
                root.band = { x0: p.x, y0: p.y, x1: mouse.x, y1: mouse.y, allFrames: !!(mouse.modifiers & Qt.ControlModifier) };
            }
        }
        onReleased: (mouse) => {
            const p = root.pending;
            root.pending = null;
            if (!p) return;
            const shift = !!(mouse.modifiers & Qt.ShiftModifier);
            if (root.band) {
                const b = root.band;
                root.searchAdds = shift;
                // Over every frame as the box showed it, also when Ctrl was let go of before the button
                root.search = controller.find_optical_tracks_in_rect(root.timestampUs, b.x0 / width, b.y0 / height, b.x1 / width, b.y1 / height, b.allFrames, root.stabilized);
                cursorShape = Qt.BusyCursor;
                return;
            }
            const id = root.pointAt(mouse.x, mouse.y);
            if (id < 0) {
                if (!shift) root.setSelection([], false);
            } else if (shift) {
                root.toggle(id);
            } else {
                root.setSelection([id], false);
            }
        }
    }

    // Delete, Backspace, Escape and Ctrl+Z/A belong to the selection here, not to the shortcuts of the window
    Keys.onShortcutOverride: (event) => {
        const ctrl = !!(event.modifiers & Qt.ControlModifier);
        if ((root.search && event.key === Qt.Key_Escape) || (root.selectionCount > 0 && [Qt.Key_Escape, Qt.Key_Backspace, Qt.Key_Delete].includes(event.key)) ||
            (ctrl && event.key === Qt.Key_Z && root.history.length > 0) || (ctrl && event.key === Qt.Key_A)) event.accepted = true;
    }
    Keys.onPressed: (event) => {
        const ctrl = !!(event.modifiers & Qt.ControlModifier);
        if (root.search && event.key === Qt.Key_Escape) {
            root.cancelSearch();
        } else if (ctrl && event.key === Qt.Key_Z && root.history.length > 0) {
            root.undo();
        } else if (ctrl && event.key === Qt.Key_A) {
            root.setSelection(root.tracksHere(null), false);
        } else if (root.selectionCount > 0 && (event.key === Qt.Key_Delete || event.key === Qt.Key_Backspace)) {
            root.removeSelected();
        } else if (root.selectionCount > 0 && event.key === Qt.Key_Escape) {
            root.setSelection([], false);
        } else {
            return;
        }
        event.accepted = true;
    }

    Menu {
        id: pointMenu;
        font.pixelSize: 11.5 * dpiScale;
        Action {
            text: qsTr("Remove selected tracks (%1)").arg(root.selectionCount);
            iconName: "bin;#f67575";
            enabled: root.selectionCount > 0;
            onTriggered: root.removeSelected();
        }
        Action {
            text: qsTr("Undo");
            iconName: "undo";
            enabled: root.history.length > 0;
            onTriggered: root.undo();
        }
        Action {
            text: qsTr("Select all in this frame");
            onTriggered: root.setSelection(root.tracksHere(null), false);
        }
        Action {
            // Downweighted and rejected: what the fit already took for something moving
            text: qsTr("Select yellow and red in this frame");
            onTriggered: root.setSelection(root.tracksHere([1, 2]), false);
        }
        Action {
            text: qsTr("Clear selection");
            enabled: root.selectionCount > 0;
            onTriggered: root.setSelection([], false);
        }
    }
}
