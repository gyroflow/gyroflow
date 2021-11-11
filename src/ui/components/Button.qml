import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC
import QtQuick.Controls.Material 2.15 as QQCM

QQC.Button {
    id: root;

    property bool accent: false;
    property color textColor: root.accent? styleTextColorOnAccent : styleTextColor;
    QQCM.Material.foreground: textColor;

    height: 35 * dpiScale;
    leftPadding: 15 * dpiScale;
    rightPadding: 15 * dpiScale;
    topPadding: 8 * dpiScale;
    bottomPadding: 8 * dpiScale;
    font.pixelSize: 14 * dpiScale;
    font.family: styleFont;
    hoverEnabled: enabled;

    background: Rectangle {
        color: root.accent? root.hovered? Qt.lighter(styleAccentColor, 1.1) : styleAccentColor : root.hovered? Qt.lighter(styleButtonColor, 1.2) : styleButtonColor;
        opacity: root.down || !parent.enabled? 0.75 : 1.0;
        Ease on opacity { duration: 100; }
        radius: 6 * dpiScale;
        anchors.fill: parent;
        border.width: style === "light"? (1 * dpiScale) : 0;
        border.color: "#cccccc";
    }

    scale: root.down? 0.970 : 1.0;
    Ease on scale { }
    font.capitalization: Font.Normal;

    property alias tooltip: tt.text;
    ToolTip { id: tt; visible: text.length > 0 && root.hovered; }
}
