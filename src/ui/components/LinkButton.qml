import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC
import QtQuick.Controls.Material 2.15 as QQCM


QQC.Button {
    id: root;

    property color textColor: styleAccentColor;
    QQCM.Material.foreground: textColor;

    font.pixelSize: 12 * dpiScale;
    font.family: styleFont;
    font.underline: true;
    font.capitalization: Font.Normal
    hoverEnabled: enabled;

    leftPadding: 15 * dpiScale;
    rightPadding: 15 * dpiScale;
    topPadding: 4 * dpiScale;
    bottomPadding: 5 * dpiScale;

    background: Rectangle {
        color: root.hovered? Qt.lighter(styleButtonColor, 1.2) : "transparent";
        opacity: root.down || !parent.enabled? 0.1 : 0.3;
        Ease on opacity { duration: 100; }
        radius: 5 * dpiScale;
        anchors.fill: parent;
    }

    MouseArea { anchors.fill: parent; acceptedButtons: Qt.NoButton; cursorShape: Qt.PointingHandCursor; }

    property alias tooltip: tt.text;
    ToolTip { id: tt; visible: text.length > 0 && root.hovered; }
}
