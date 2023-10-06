// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2023 Adrian <adrian.eddy at gmail>

import QtQuick
import QtQuick.Window
import QtQuick.Controls as QQC
import QtQuick.Controls.Material
import Qt.labs.settings

import "components/"

Window {
    id: main_window;
    width:  isMobile? Screen.desktopAvailableWidth  : Math.min(Screen.width, 1650 * dpiScale);
    height: isMobile? Screen.desktopAvailableHeight : Math.min(Screen.height, 950 * dpiScale);
    minimumWidth: 900 * dpiScale;
    minimumHeight: 400 * dpiScale;
    visible: true;
    color: styleBackground;
    property var safeAreaMargins: ({});
    onWidthChanged: updateMargins.start();
    onHeightChanged: updateMargins.start();
    Timer {
        id: updateMargins;
        interval: 100;
        onTriggered: main_window.safeAreaMargins = ui_tools.get_safe_area_margins(main_window);
    }

    title: "Gyroflow v" + version;

    onVisibilityChanged: {
        Qt.callLater(() => {
            if (main_window.visibility != 0)
                sett.visibility = main_window.visibility;
        });
    }

    Settings {
        id: sett;
        property alias x: main_window.x;
        property alias y: main_window.y;
        property alias width: main_window.width;
        property alias height: main_window.height;
        property int visibility: 0;
    }

    Material.theme: Material.Dark;
    Material.accent: Material.Blue;

    function getApp(): App {
        for (let i = 0; i < contentItem.children.length; ++i) {
            let x = contentItem.children[i];
            if (x instanceof Loader) x = x.item;
            if (x.objectName == "App") return x;
        }
        return null;
    }

    Component.onCompleted: {
        ui_tools.set_icon(main_window);
             if (!isMobile && sett.visibility == Window.FullScreen) main_window.showFullScreen();
        else if (!isMobile && sett.visibility == Window.Maximized)  main_window.showMaximized();
        else if (!isMobile) {
            Qt.callLater(() => {
                width = width + 1;
                height = height;
            });
        } else {
            Qt.callLater(() => { main_window.showFullScreen(); });
        }
        updateMargins.start();
    }
    property bool isLandscape: width > height;

    property bool closeConfirmationModal: false;
    property bool closeConfirmed: false;
    onClosing: (close) => {
        let app = getApp();
        if (app) {
            close.accepted = closeConfirmed || !app.wasModified;
            if (close.accepted) {
                ui_tools.closing();
                main_controller.cancel_current_operation();
                if (typeof calib_controller !== "undefined")
                    calib_controller.cancel_current_operation();
            }
            if (!close.accepted && !closeConfirmationModal) {
                closeConfirmationModal = true;
                app.messageBox(Modal.Question, qsTr("Are you sure you want to exit?"), [
                    { text: qsTr("Yes"), accent: true, clicked: () => { main_window.closeConfirmed = true; main_window.close(); } },
                    { text: qsTr("No"), clicked: () => { main_window.closeConfirmationModal = false; } }
                ]);
            }
        }
    }

    Rectangle {
        id: libg;
        anchors.fill: loadingImage;
        anchors.margins: -20 * dpiScale;
        radius: 10 * dpiScale;
        z: 9998;
        opacity: 0.5;
        Ease on opacity { duration: 1000; }
        visible: opacity > 0;
        color: styleBackground;
    }
    Image {
        id: loadingImage;
        source: "qrc:/resources/logo" + (style === "dark"? "_white" : "_black") + ".svg";
        sourceSize.width: Math.min(400 * dpiScale, parent.width * 0.7);
        opacity: 0;
        YAnimator       on y       { id: liy; from: -1000; to: -1000; duration: 1000; easing.type: Easing.OutExpo; }
        OpacityAnimator on opacity { id: lio; from: 0; to: 1; duration: 1000; easing.type: Easing.OutExpo; }
        anchors.horizontalCenter: parent.horizontalCenter;
        z: 9999;
        onHeightChanged: updateYAnim(loadingIndicator.y, height);
        function updateYAnim(indicatorY: real, imageHeight: real) {
            liy.stop();
            liy.from = indicatorY - imageHeight - 10 * dpiScale;
            liy.to = indicatorY - imageHeight - 30 * dpiScale;
            liy.restart();
        }
    }
    Loader {
        id: appLoader;
        objectName: "AppLoader";
        anchors.fill: parent;
        asynchronous: true;
        opacity: appLoader.status == Loader.Ready? 1 : 0.5;
        onStatusChanged: {
            if (status == Loader.Ready) {
                Qt.callLater(item.isMobileLayoutChanged);
                Qt.callLater(item.isLandscapeChanged);
            }
        }
        Ease on opacity { }
        sourceComponent: Component {
            App { objectName: "App"; }
        }
    }
    QQC.BusyIndicator {
        id: loadingIndicator;
        anchors.centerIn: parent;
        running: appLoader.status != Loader.Ready;
        onYChanged: loadingImage.updateYAnim(y, loadingImage.height);
        onRunningChanged: if (!running) { destroy(700); lio.stop(); lio.from = 1; lio.to = 0; lio.restart(); libg.opacity = 0; libg.destroy(1000); loadingImage.destroy(1000); }
    }
}
