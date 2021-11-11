import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC

Text {
    leftPadding: 10 * dpiScale;
    color: styleTextColor;
    font.pixelSize: 12 * dpiScale;
    font.family: styleFont;
    opacity: enabled? 1.0 : 0.6;
}
