import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC

Row {
    id: tl;
    property var model: ({});
    property alias col1: col1;
    property alias col2: col2;
    width: parent.width;

    function updateEntry(key, value) {
        model[key] = value;
        modelChanged();
    }
    Column {
        id: col1;
        spacing: 8 * dpiScale;
        Repeater {
            model: Object.keys(tl.model);
            BasicText { text: qsTr(modelData) + ":"; anchors.right: parent.right; leftPadding: 0; }
        }
    }
    Column {
        id: col2;
        spacing: 8 * dpiScale;
        Repeater {
            model: Object.values(tl.model);
            BasicText { text: modelData; font.bold: true; }
        }
    }
}
