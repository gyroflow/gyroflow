// SPDX-License-Identifier: GPL-3.0-or-later
// Put input.png and atlas.png beside this harness; adjust size/controls.
// Run from the fixture output folder, or change the output filename below.
import QtQuick
import QtQuick.Window
Window {
    width: 128; height: 128; visible: true;
    ShaderEffect {
        id: result; width: parent.width; height: parent.height;
        property var source: Image { source: "input.png"; visible: false; smooth: false; }
        property var lutTexture: Image { source: "atlas.png"; visible: false; smooth: false; }
        property real brightness: 0.1;
        property real contrast: 0.2;
        property real lutSize: 33;
        fragmentShader: "../../src/qt_gpu/compiled/color_preview.frag.qsb";
    }
    Timer {
        interval: 1000; running: true;
        onTriggered: result.grabToImage(function(image) {
            if (!image.saveToFile("actual.png")) console.error("Could not save the shader result");
            Qt.quit();
        });
    }
}
