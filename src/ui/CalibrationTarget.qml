import QtQuick 2.15
import QtQuick.Window 2.10

Window {
    id: root;
    width: 1000;
    height: 571;
    visible: true;
    visibility: Window.Maximized;
    color: "#ffffff";

    property int columns: 14;
    property int rows: 8;
    property real tileSize: Math.min(root.height / (rows + 2), root.width / (columns + 2));

    title: qsTr("Calibration target") + ` (${columns} x ${rows})`;

    Column {
        anchors.centerIn: parent;
        Repeater {
            model: (root.rows + 1);
            Row {
                property int colIndex: index;
                Repeater {
                    model: (root.columns + 1);
                    Rectangle {
                        width: root.tileSize;
                        height: width;
                        color: ((colIndex % 2 == 0)? (index % 2 != 0) : (index % 2 == 0))? "white" : "black";
                    }
                }
            }
        }
    }
}
