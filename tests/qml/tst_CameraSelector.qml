import QtQuick
import QtTest
import "../../src/ui/components" as Components

Item {
    width: 360;
    height: 420;
    property real dpiScale: 1;
    property bool isMobile: false;
    property string style: "dark";
    property string styleFont: "Arial";
    property color styleTextColor: "white";
    property color styleAccentColor: "#50b0e0";
    property color styleButtonColor: "#303030";
    property color styleBackground2: "#202020";
    property color stylePopupBorder: "#606060";
    property color styleHighlightColor: "#505050";

    Components.CameraSelector {
        id: selector;
        width: 300;
        fieldLabel: "Camera brand";
        model: ["Other", "Sony", "Canon", "Panasonic"];
        currentIndex: 1;
    }
    SignalSpy { id: activation; target: selector; signalName: "activated"; }
    TestCase {
        name: "CameraSelector";
        when: windowShown;
        function init() {
            selector.model = ["Other", "Sony", "Canon", "Panasonic"];
            selector.currentIndex = 1;
            activation.clear();
        }
        function cleanup() { selector.popup.close(); tryCompare(selector.popup, "visible", false); }
        function test_filter_does_not_change_selection() {
            selector.popup.open();
            tryCompare(selector.popup, "opened", true);
            const field = findChild(selector.popup, "cameraSelectorFilter");
            verify(field !== null);
            tryCompare(field, "activeFocus", true);
            field.text = "CAN";
            compare(selector.popup.filteredIndices, [2]);
            tryCompare(findChild(selector.popup, "cameraSelectorOptions"), "currentIndex", 0);
            compare(selector.currentIndex, 1);
            compare(activation.count, 0);
            keyClick(Qt.Key_Return);
            compare(selector.currentIndex, 2);
            compare(activation.count, 1);
        }
        function test_unmatched_filter_cannot_select_invalid_row() {
            selector.popup.open();
            tryCompare(selector.popup, "opened", true);
            findChild(selector.popup, "cameraSelectorFilter").text = "missing";
            compare(selector.popup.filteredIndices.length, 0);
            keyClick(Qt.Key_Return);
            compare(selector.currentIndex, 1);
            compare(activation.count, 0);
        }
        function test_keyboard_opens_search_and_model_reset_stays_valid() {
            selector.forceActiveFocus();
            keyClick(Qt.Key_Return);
            tryCompare(selector.popup, "opened", true);
            selector.model = ["Other", "New camera"];
            findChild(selector.popup, "cameraSelectorFilter").text = "new";
            keyClick(Qt.Key_Return);
            compare(selector.currentIndex, 1);
            compare(selector.currentText, "New camera");
        }
    }
}
