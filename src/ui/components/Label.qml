import QtQuick 2.15

Grid {
    id: root;

    enum LabelPosition { Top, Left }

    property int position: Label.Top;
    default property alias data: inner.data;
    property alias text: t.text;
    property alias inner: inner;

    rows:    position === Label.Top? 2 : 1;
    columns: position === Label.Top? 1 : 2;
    spacing: 8 * dpiScale;
    width: parent.width;

    BasicText {
        id: t;
        leftPadding: 0;
        verticalAlignment: Text.AlignVCenter;
        height: root.position === Label.Top? undefined : inner.height;
    }

    Item {
        id: inner;
        width: parent.width - (root.position === Label.Top? 0 : t.width + root.spacing);
        height: children[0].height + 2 * dpiScale;
    }
}
