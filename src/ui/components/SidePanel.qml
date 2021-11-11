import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC

ResizablePanel {
    height: parent.height;
    default property alias data: col.data;
    implicitWidth: 300 * dpiScale;
    property real topPadding: 0;
    property real bottomPadding: 0;

    Flickable {
        width: parent.width - 2*x;
        x: 4 * dpiScale;
        y: topPadding;
        height: parent.height - y - parent.bottomPadding;
        clip: true;
        QQC.ScrollIndicator.vertical: QQC.ScrollIndicator { padding: 0; }

        contentHeight: col.height;
        contentWidth: width;
        Column {
            id: col;
            spacing: 5 * dpiScale;
            width: parent.width;
        }
    }
}
