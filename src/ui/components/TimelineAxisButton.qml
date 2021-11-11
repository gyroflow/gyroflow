import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC

QQC.AbstractButton {
    id: root;
    height: 25 * dpiScale;
    width: 25 * dpiScale;
    font.pixelSize: 10 * dpiScale;
    font.family: styleFont;

    background: Rectangle {
        color: if (style === "light") {
                   return root.checked? Qt.lighter(styleAccentColor) : root.hovered? Qt.darker(styleButtonColor, 1.2) : styleHrColor;
               } else {
                   return root.checked? Qt.darker(styleAccentColor) : root.hovered? Qt.lighter(styleButtonColor, 1.2) : Qt.darker(styleButtonColor, 1.2);
               }

        opacity: root.down || !parent.enabled? 0.75 : 1.0;
        Ease on opacity { duration: 100; }
        radius: 3 * dpiScale;
    }

    contentItem: BasicText {
        text: root.text;
        color: style === "light"? (root.checked? styleTextColorOnAccent : styleTextColor) : styleTextColor;
        opacity: root.checked? 1.0 : 0.5;
        font: root.font;
        leftPadding: 0;
        horizontalAlignment: Text.AlignHCenter;
        verticalAlignment: Text.AlignVCenter;
    }
    onClicked: checked = !checked;
}
