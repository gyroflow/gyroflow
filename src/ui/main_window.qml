import QtQuick 2.15
import QtQuick.Window 2.10
import QtQuick.Controls.Material 2.12

Window {
    width: 1450;
    height: 800;
    visible: true;
    color: styleBackground;

    title: "Gyroflow"
    
    Material.theme: Material.Dark;
    Material.accent: Material.Blue;

    App { objectName: "App" }
}
