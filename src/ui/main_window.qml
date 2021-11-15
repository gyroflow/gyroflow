import QtQuick 2.15
import QtQuick.Window 2.10
import QtQuick.Controls.Material 2.12

Window {
    id: main_window;
    width: 1450;
    height: 800;
    visible: true;
    color: styleBackground;

    title: "Gyroflow"
    
    Material.theme: Material.Dark;
    Material.accent: Material.Blue;

    property QtObject app: contentItem.children[0];

    property bool closeConfirmationModal: false;
    onClosing: (close) => {
        if (!closeConfirmationModal) {
            app.messageBox(qsTr("Are you sure you want to exit?"), [
                { text: qsTr("Yes"), accent: true, clicked: () => main_window.close() },
                { text: qsTr("No"), clicked: () => main_window.closeConfirmationModal = false },
            ]);
            close.accepted = false;
            closeConfirmationModal = true;
        }
    }
    
    App { objectName: "App" }
}
