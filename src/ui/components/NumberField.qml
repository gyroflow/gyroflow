import QtQuick 2.15
import QtQuick.Controls 2.15 as QQC

TextField {
    id: root;
    property int precision: 0;
    property string unit: "";
    property real value: 0;
    property bool preventChange: false;
    property alias from: validator.bottom;
    property alias to: validator.top;

    Keys.onDownPressed: (e) => {
             if (e.modifiers & Qt.AltModifier) value -= 0.001;
        else if (e.modifiers & Qt.ShiftModifier) value -= 1;
        else value -= 0.01;
    }
    Keys.onUpPressed:(e) => {
             if (e.modifiers & Qt.AltModifier) value += 0.001;
        else if (e.modifiers & Qt.ShiftModifier) value += 1;
        else value += 0.01;
    }

    onValueChanged: {
        if (preventChange) return;
        text = value.toLocaleString(Qt.locale(), "f", precision);
    }
    onTextChanged: {
        preventChange = true;
        value = Number.fromLocaleString(Qt.locale(), text);
        preventChange = false;
    }
    Component.onCompleted: valueChanged();
    onAccepted: valueChanged();
    onFocusChanged: if (!activeFocus) valueChanged();

    Rectangle {
        visible: !root.acceptableInput;
        anchors.fill: parent;
        color: "transparent";
        radius: root.background.radius;
        border.color: "#c33838";
        border.width: 1 * dpiScale;
    }

    inputMethodHints: Qt.ImhPreferNumbers | Qt.ImhFormattedNumbersOnly

    validator: DoubleValidator { id: validator; decimals: root.precision }

    BasicText {
        visible: !!root.unit;
        x: parent.contentWidth;
        text: root.unit;
        height: parent.height;
        verticalAlignment: Text.AlignVCenter;
    }
}
