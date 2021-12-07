import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC

InfoMessage {
    property bool show: false;
    visible: opacity > 0;
    opacity: show? 1 : 0;
    Ease on opacity { }
    height: (t.height + 10 * dpiScale) * opacity - parent.spacing * (1.0 - opacity);
    t.font.pixelSize: 12 * dpiScale;
    t.x: 5 * dpiScale;
}
