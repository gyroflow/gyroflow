import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC

import "." as UI;

Row {
    id: tl;
    property var model: ({});
    width: parent.width;

    function updateEntry(key, value) {
        model[key] = value;
        modelChanged();
    }
    Column {
        spacing: 8 * dpiScale;
        Repeater {
            model: Object.keys(tl.model);
            UI.BasicText { text: qsTr(modelData) + ":"; anchors.right: parent.right; leftPadding: 0; }
        }
    }
    Column {
        spacing: 8 * dpiScale;
        Repeater {
            model: Object.values(tl.model);
            UI.BasicText { text: modelData; font.bold: true; }
        }
    }
}
