#!/bin/bash

PROJECT_DIR="/Users/admin/gyroflow"

CARGO_TARGET="$PROJECT_DIR/target/release"
QT_DIR="$PROJECT_DIR/ext/6.2.2/macos"
OPENCV_DIR="$PROJECT_DIR/ext/vcpkg/installed"
FFMPEG_DIR="$PROJECT_DIR/ext/ffmpeg-4.4-macOS-default"

#cargo build --release --target=x86_64-apple-darwin
#cargo build --release --target=aarch64-apple-darwin
#@lipo target/{x86_64,aarch64}-apple-darwin/release/$(TARGET) -create -output $(APP_BINARY)

if [ "$1" == "deploy" ]; then
    mkdir -p "$PROJECT_DIR/_deployment/_binaries/mac64"
    CARGO_TARGET="$PROJECT_DIR/_deployment/_binaries/mac64/Gyroflow.app/Contents/MacOS"
    cp -Rf "$PROJECT_DIR/_deployment/mac/Gyroflow.app"    "$PROJECT_DIR/_deployment/_binaries/mac64/"
    cp -f  "$PROJECT_DIR/target/release/gyroflow"         "$PROJECT_DIR/_deployment/_binaries/mac64/Gyroflow.app/Contents/MacOS/"
    cp -Rf "$PROJECT_DIR/target/Frameworks/mdk.framework" "$PROJECT_DIR/_deployment/_binaries/mac64/Gyroflow.app/Contents/Frameworks/mdk.framework"
    cp -Rf "$PROJECT_DIR/resources/camera_presets"        "$PROJECT_DIR/_deployment/_binaries/mac64/Gyroflow.app/Contents/Resources/"
fi

cp -af "$QT_DIR/lib/QtCore.framework"                     "$CARGO_TARGET/../Frameworks/"
cp -af "$QT_DIR/lib/QtDBus.framework"                     "$CARGO_TARGET/../Frameworks/"
cp -af "$QT_DIR/lib/QtGui.framework"                      "$CARGO_TARGET/../Frameworks/"
cp -af "$QT_DIR/lib/QtLabsSettings.framework"             "$CARGO_TARGET/../Frameworks/"
cp -af "$QT_DIR/lib/QtNetwork.framework"                  "$CARGO_TARGET/../Frameworks/"
cp -af "$QT_DIR/lib/QtOpenGL.framework"                   "$CARGO_TARGET/../Frameworks/"
cp -af "$QT_DIR/lib/QtQml.framework"                      "$CARGO_TARGET/../Frameworks/"
cp -af "$QT_DIR/lib/QtQmlModels.framework"                "$CARGO_TARGET/../Frameworks/"
cp -af "$QT_DIR/lib/QtQmlWorkerScript.framework"          "$CARGO_TARGET/../Frameworks/"
cp -af "$QT_DIR/lib/QtQuick.framework"                    "$CARGO_TARGET/../Frameworks/"
cp -af "$QT_DIR/lib/QtQuickControls2.framework"           "$CARGO_TARGET/../Frameworks/"
cp -af "$QT_DIR/lib/QtQuickControls2Impl.framework"       "$CARGO_TARGET/../Frameworks/"
cp -af "$QT_DIR/lib/QtQuickDialogs2.framework"            "$CARGO_TARGET/../Frameworks/"
cp -af "$QT_DIR/lib/QtQuickDialogs2QuickImpl.framework"   "$CARGO_TARGET/../Frameworks/"
cp -af "$QT_DIR/lib/QtQuickDialogs2Utils.framework"       "$CARGO_TARGET/../Frameworks/"
cp -af "$QT_DIR/lib/QtQuickTemplates2.framework"          "$CARGO_TARGET/../Frameworks/"
cp -af "$QT_DIR/lib/QtSvg.framework"                      "$CARGO_TARGET/../Frameworks/"
cp -af "$QT_DIR/lib/QtWidgets.framework"                  "$CARGO_TARGET/../Frameworks/"

if [ "$1" == "deploy" ]; then
    CARGO_TARGET="$PROJECT_DIR/_deployment/_binaries/mac64/Gyroflow.app/Contents/Resources/qml"
fi

mkdir -p "$CARGO_TARGET/Qt/labs/settings/"
mkdir -p "$CARGO_TARGET/QtQml/WorkerScript/"
mkdir -p "$CARGO_TARGET/QtQuick/Controls/impl/"
mkdir -p "$CARGO_TARGET/QtQuick/Controls/macOS/"
mkdir -p "$CARGO_TARGET/QtQuick/Controls/Basic/impl/"
mkdir -p "$CARGO_TARGET/QtQuick/Controls/Material/impl/"
mkdir -p "$CARGO_TARGET/QtQuick/Layouts/"
mkdir -p "$CARGO_TARGET/QtQuick/Window/"
mkdir -p "$CARGO_TARGET/QtQuick/Templates/"
mkdir -p "$CARGO_TARGET/QtQuick/Dialogs/quickimpl/qml/+Material/"

if [ "$1" == "deploy" ]; then
    CARGO_TARGET="$PROJECT_DIR/_deployment/_binaries/mac64/Gyroflow.app/Contents/Resources/qml"
fi
cp -f $QT_DIR/qml/Qt/labs/settings/qmldir                                                         "$CARGO_TARGET/Qt/labs/settings/"
cp -f $QT_DIR/qml/Qt/labs/settings/libqmlsettingsplugin.dylib                                     "$CARGO_TARGET/Qt/labs/settings/"
cp -f $QT_DIR/qml/QtQml/qmldir                                                                    "$CARGO_TARGET/QtQml/"
cp -f $QT_DIR/qml/QtQml/libqmlplugin.dylib                                                        "$CARGO_TARGET/QtQml/"
cp -f $QT_DIR/qml/QtQml/WorkerScript/libworkerscriptplugin.dylib                                  "$CARGO_TARGET/QtQml/WorkerScript/"
cp -f $QT_DIR/qml/QtQml/WorkerScript/qmldir                                                       "$CARGO_TARGET/QtQml/WorkerScript/"
cp -f $QT_DIR/qml/QtQuick/qmldir                                                                  "$CARGO_TARGET/QtQuick"
cp -f $QT_DIR/qml/QtQuick/Controls/impl/qmldir                                                    "$CARGO_TARGET/QtQuick/Controls/impl/"
cp -f $QT_DIR/qml/QtQuick/Controls/impl/libqtquickcontrols2implplugin.dylib                       "$CARGO_TARGET/QtQuick/Controls/impl/"
cp -f $QT_DIR/qml/QtQuick/Controls/qmldir                                                         "$CARGO_TARGET/QtQuick/Controls/"
cp -f $QT_DIR/qml/QtQuick/Controls/macOS/*.qml                                                    "$CARGO_TARGET/QtQuick/Controls/macOS/"
cp -f $QT_DIR/qml/QtQuick/Controls/macOS/qmldir                                                   "$CARGO_TARGET/QtQuick/Controls/macOS/"
cp -f $QT_DIR/qml/QtQuick/Controls/macOS/libqtquickcontrols2macosstyleplugin.dylib                "$CARGO_TARGET/QtQuick/Controls/macOS/"
cp -f $QT_DIR/qml/QtQuick/Controls/Basic/*.qml                                                    "$CARGO_TARGET/QtQuick/Controls/Basic/"
cp -f $QT_DIR/qml/QtQuick/Controls/Basic/impl/qmldir                                              "$CARGO_TARGET/QtQuick/Controls/Basic/impl/"
cp -f $QT_DIR/qml/QtQuick/Controls/Basic/impl/libqtquickcontrols2basicstyleimplplugin.dylib       "$CARGO_TARGET/QtQuick/Controls/Basic/impl/"
cp -f $QT_DIR/qml/QtQuick/Controls/Basic/qmldir                                                   "$CARGO_TARGET/QtQuick/Controls/Basic/"
cp -f $QT_DIR/qml/QtQuick/Controls/Basic/libqtquickcontrols2basicstyleplugin.dylib                "$CARGO_TARGET/QtQuick/Controls/Basic/"
cp -f $QT_DIR/qml/QtQuick/Controls/Material/impl/*.qml                                            "$CARGO_TARGET/QtQuick/Controls/Material/impl/"
cp -f $QT_DIR/qml/QtQuick/Controls/Material/impl/qmldir                                           "$CARGO_TARGET/QtQuick/Controls/Material/impl/"
cp -f $QT_DIR/qml/QtQuick/Controls/Material/impl/libqtquickcontrols2materialstyleimplplugin.dylib "$CARGO_TARGET/QtQuick/Controls/Material/impl/"
cp -f $QT_DIR/qml/QtQuick/Controls/Material/*.qml                                                 "$CARGO_TARGET/QtQuick/Controls/Material/"
cp -f $QT_DIR/qml/QtQuick/Controls/Material/qmldir                                                "$CARGO_TARGET/QtQuick/Controls/Material/"
cp -f $QT_DIR/qml/QtQuick/Controls/Material/libqtquickcontrols2materialstyleplugin.dylib          "$CARGO_TARGET/QtQuick/Controls/Material/"
cp -f $QT_DIR/qml/QtQuick/Controls/libqtquickcontrols2plugin.dylib                                "$CARGO_TARGET/QtQuick/Controls/"
cp -f $QT_DIR/qml/QtQuick/Layouts/qmldir                                                          "$CARGO_TARGET/QtQuick/Layouts/"
cp -f $QT_DIR/qml/QtQuick/Layouts/libqquicklayoutsplugin.dylib                                    "$CARGO_TARGET/QtQuick/Layouts/"
cp -f $QT_DIR/qml/QtQuick/libqtquick2plugin.dylib                                                 "$CARGO_TARGET/QtQuick/"
cp -f $QT_DIR/qml/QtQuick/Window/qmldir                                                           "$CARGO_TARGET/QtQuick/Window/"
cp -f $QT_DIR/qml/QtQuick/Window/libquickwindowplugin.dylib                                       "$CARGO_TARGET/QtQuick/Window/"
cp -f $QT_DIR/qml/QtQuick/Templates/qmldir                                                        "$CARGO_TARGET/QtQuick/Templates/"
cp -f $QT_DIR/qml/QtQuick/Templates/libqtquicktemplates2plugin.dylib                              "$CARGO_TARGET/QtQuick/Templates/"
cp -f $QT_DIR/qml/QtQuick/Dialogs/qmldir                                                          "$CARGO_TARGET/QtQuick/Dialogs/"
cp -f $QT_DIR/qml/QtQuick/Dialogs/libqtquickdialogsplugin.dylib                                   "$CARGO_TARGET/QtQuick/Dialogs/"
cp -f $QT_DIR/qml/QtQuick/Dialogs/quickimpl/qmldir                                                "$CARGO_TARGET/QtQuick/Dialogs/quickimpl/"
cp -f $QT_DIR/qml/QtQuick/Dialogs/quickimpl/qml/*.qml                                             "$CARGO_TARGET/QtQuick/Dialogs/quickimpl/qml/"
cp -f $QT_DIR/qml/QtQuick/Dialogs/quickimpl/qml/+Material/*.qml                                   "$CARGO_TARGET/QtQuick/Dialogs/quickimpl/qml/+Material/"
cp -f $QT_DIR/qml/QtQuick/Dialogs/quickimpl/libqtquickdialogs2quickimplplugin.dylib               "$CARGO_TARGET/QtQuick/Dialogs/quickimpl/"

if [ "$1" == "deploy" ]; then
    CARGO_TARGET="$PROJECT_DIR/_deployment/_binaries/mac64/Gyroflow.app/Contents/PlugIns"
fi
mkdir -p "$CARGO_TARGET/iconengines/"
mkdir -p "$CARGO_TARGET/imageformats/"
mkdir -p "$CARGO_TARGET/platforms/"
cp -f $QT_DIR/plugins/iconengines/libqsvgicon.dylib                                               "$CARGO_TARGET/iconengines/"
cp -f $QT_DIR/plugins/imageformats/libqsvg.dylib                                                  "$CARGO_TARGET/imageformats/"
cp -f $QT_DIR/plugins/platforms/libqcocoa.dylib                                                   "$CARGO_TARGET/platforms/"

if [ "$1" == "deploy" ]; then
    ln -sf /Applications "$PROJECT_DIR/_deployment/_binaries/mac64/Applications"
    hdiutil create "$PROJECT_DIR/_deployment/_binaries/Gyroflow-mac64.dmg" -volname "Gyroflow v0.4.0" -fs HFS+ -srcfolder "$PROJECT_DIR/_deployment/_binaries/mac64/" -ov -format UDZO -imagekey zlib-level=9
fi
