#!/bin/bash

: "${PROJECT_DIR:=$(readlink -f $(dirname $(readlink -f $0))/..)}"
: "${CARGO_TARGET:=$PROJECT_DIR/target/deploy}"
: "${QT_DIR:=$PROJECT_DIR/ext/6.2.3/gcc_64}"
: "${FFMPEG_DIR:=$PROJECT_DIR/ext/ffmpeg-5.0-linux-clang-gpl-lite}"
: "${VCPKG_ROOT:=$PROJECT_DIR/ext/vcpkg}"

if [ "$1" == "build-docker" ]; then
    sudo docker run -v $PROJECT_DIR:$PROJECT_DIR -v $HOME/.cargo:/root/.cargo debian:10 bash -c "
        apt update
        echo 'debconf debconf/frontend select Noninteractive' | debconf-set-selections
        apt install -y sudo dialog apt-utils
        export RUNLEVEL=1
        export VCPKG_ROOT=$VCPKG_ROOT
        cd $PROJECT_DIR/ext
        ./install-deps-linux.sh docker
        curl https://sh.rustup.rs -sSf | sh -s -- -y --profile minimal
        source \$HOME/.cargo/env
        export FFMPEG_DIR=$FFMPEG_DIR
        export GITHUB_RUN_NUMBER=$GITHUB_RUN_NUMBER
        export OPENCV_LINK_PATHS=\$VCPKG_ROOT/installed/x64-linux-release/lib
        export OPENCV_INCLUDE_PATHS=\$VCPKG_ROOT/installed/x64-linux-release/include/

        export PATH=\"$QT_DIR/bin:\$PATH\"
        export OPENCV_LINK_LIBS=\"opencv_core,opencv_calib3d,opencv_features2d,opencv_imgproc,opencv_video,opencv_flann\"
        cd $PROJECT_DIR
        echo \$FFMPEG_DIR
        ls -l \$FFMPEG_DIR
        cargo build --profile deploy
        export PROJECT_DIR=$PROJECT_DIR
        export QT_DIR=$QT_DIR
        ./_deployment/deploy-linux.sh
    "
    stat $HOME/.cargo
    stat $PROJECT_DIR/Cargo.toml
    sudo chown -R $(stat -c "%U:%G" $PROJECT_DIR/Cargo.toml) $HOME/.cargo
    sudo chown -R $(stat -c "%U:%G" $PROJECT_DIR/Cargo.toml) $PROJECT_DIR
    exit;
fi

rm -rf "$PROJECT_DIR/_deployment/_binaries/linux64"

TARGET="$PROJECT_DIR/_deployment/_binaries/linux64"
mkdir -p $TARGET
mkdir -p $TARGET/camera_presets
mkdir -p $TARGET/lib
mkdir -p $TARGET/plugins
mkdir -p $TARGET/qml

cp -f "$QT_DIR/lib/libQt6Core.so.6"                    "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6Gui.so.6"                     "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6Quick.so.6"                   "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6Qml.so.6"                     "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6QmlCore.so.6"                 "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6LabsSettings.so.6"            "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6LabsFolderListModel.so.6"     "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6LabsQmlModels.so.6"           "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6QuickControls2.so.6"          "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6QuickControls2Impl.so.6"      "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6QuickTemplates2.so.6"         "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6QuickDialogs2.so.6"           "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6QuickDialogs2QuickImpl.so.6"  "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6QuickDialogs2Utils.so.6"      "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6QuickLayouts.so.6"            "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6Svg.so.6"                     "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6DBus.so.6"                    "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6QmlModels.so.6"               "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6QmlWorkerScript.so.6"         "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6Network.so.6"                 "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6OpenGL.so.6"                  "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6Widgets.so.6"                 "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6XcbQpa.so.6"                  "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6WaylandClient.so.6"           "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6WaylandEglClientHwIntegration.so.6" "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6EglFSDeviceIntegration.so.6"  "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6EglFsKmsSupport.so.6"         "$TARGET/lib/"
cp -f "$QT_DIR/lib/libQt6WlShellIntegration.so.6"      "$TARGET/lib/"
cp -f "$QT_DIR/lib/libicudata.so.56"                   "$TARGET/lib/"
cp -f "$QT_DIR/lib/libicuuc.so.56"                     "$TARGET/lib/"
cp -f "$QT_DIR/lib/libicui18n.so.56"                   "$TARGET/lib/"

mkdir -p "$TARGET/qml/Qt/labs/settings/"
mkdir -p "$TARGET/qml/Qt/labs/folderlistmodel/"
mkdir -p "$TARGET/qml/QtQml/WorkerScript/"
mkdir -p "$TARGET/qml/QtQuick/Controls/impl/"
mkdir -p "$TARGET/qml/QtQuick/Controls/Basic/impl/"
mkdir -p "$TARGET/qml/QtQuick/Controls/Material/impl/"
mkdir -p "$TARGET/qml/QtQuick/Layouts/"
mkdir -p "$TARGET/qml/QtQuick/Window/"
mkdir -p "$TARGET/qml/QtQuick/Templates/"
mkdir -p "$TARGET/qml/QtQuick/Dialogs/quickimpl/qml/+Material/"

cp -f $QT_DIR/qml/Qt/labs/settings/qmldir                                                        "$TARGET/qml/Qt/labs/settings/"
cp -f $QT_DIR/qml/Qt/labs/settings/libqmlsettingsplugin.so                                       "$TARGET/qml/Qt/labs/settings/"

cp -f $QT_DIR/qml/Qt/labs/folderlistmodel/qmldir                                                 "$TARGET/qml/Qt/labs/folderlistmodel/"
cp -f $QT_DIR/qml/Qt/labs/folderlistmodel/libqmlfolderlistmodelplugin.so                         "$TARGET/qml/Qt/labs/folderlistmodel/"
cp -f $QT_DIR/qml/QtQml/qmldir                                                                   "$TARGET/qml/QtQml/"
cp -f $QT_DIR/qml/QtQml/libqmlplugin.so                                                          "$TARGET/qml/QtQml/"
cp -f $QT_DIR/qml/QtQml/WorkerScript/libworkerscriptplugin.so                                    "$TARGET/qml/QtQml/WorkerScript/"
cp -f $QT_DIR/qml/QtQml/WorkerScript/qmldir                                                      "$TARGET/qml/QtQml/WorkerScript/"
cp -f $QT_DIR/qml/QtQuick/qmldir                                                                 "$TARGET/qml/QtQuick"
cp -f $QT_DIR/qml/QtQuick/Controls/impl/qmldir                                                   "$TARGET/qml/QtQuick/Controls/impl/"
cp -f $QT_DIR/qml/QtQuick/Controls/impl/libqtquickcontrols2implplugin.so                         "$TARGET/qml/QtQuick/Controls/impl/"
cp -f $QT_DIR/qml/QtQuick/Controls/qmldir                                                        "$TARGET/qml/QtQuick/Controls/"
cp -f $QT_DIR/qml/QtQuick/Controls/Basic/*.qml                                                   "$TARGET/qml/QtQuick/Controls/Basic/"
cp -f $QT_DIR/qml/QtQuick/Controls/Basic/impl/qmldir                                             "$TARGET/qml/QtQuick/Controls/Basic/impl/"
cp -f $QT_DIR/qml/QtQuick/Controls/Basic/impl/libqtquickcontrols2basicstyleimplplugin.so         "$TARGET/qml/QtQuick/Controls/Basic/impl/"
cp -f $QT_DIR/qml/QtQuick/Controls/Basic/qmldir                                                  "$TARGET/qml/QtQuick/Controls/Basic/"
cp -f $QT_DIR/qml/QtQuick/Controls/Basic/libqtquickcontrols2basicstyleplugin.so                  "$TARGET/qml/QtQuick/Controls/Basic/"
cp -f $QT_DIR/qml/QtQuick/Controls/Material/impl/*.qml                                           "$TARGET/qml/QtQuick/Controls/Material/impl/"
cp -f $QT_DIR/qml/QtQuick/Controls/Material/impl/qmldir                                          "$TARGET/qml/QtQuick/Controls/Material/impl/"
cp -f $QT_DIR/qml/QtQuick/Controls/Material/impl/libqtquickcontrols2materialstyleimplplugin.so   "$TARGET/qml/QtQuick/Controls/Material/impl/"
cp -f $QT_DIR/qml/QtQuick/Controls/Material/*.qml                                                "$TARGET/qml/QtQuick/Controls/Material/"
cp -f $QT_DIR/qml/QtQuick/Controls/Material/qmldir                                               "$TARGET/qml/QtQuick/Controls/Material/"
cp -f $QT_DIR/qml/QtQuick/Controls/Material/libqtquickcontrols2materialstyleplugin.so            "$TARGET/qml/QtQuick/Controls/Material/"
cp -f $QT_DIR/qml/QtQuick/Controls/libqtquickcontrols2plugin.so                                  "$TARGET/qml/QtQuick/Controls/"
cp -f $QT_DIR/qml/QtQuick/Layouts/qmldir                                                         "$TARGET/qml/QtQuick/Layouts/"
cp -f $QT_DIR/qml/QtQuick/Layouts/libqquicklayoutsplugin.so                                      "$TARGET/qml/QtQuick/Layouts/"
cp -f $QT_DIR/qml/QtQuick/libqtquick2plugin.so                                                   "$TARGET/qml/QtQuick/"
cp -f $QT_DIR/qml/QtQuick/Window/qmldir                                                          "$TARGET/qml/QtQuick/Window/"
cp -f $QT_DIR/qml/QtQuick/Window/libquickwindowplugin.so                                         "$TARGET/qml/QtQuick/Window/"
cp -f $QT_DIR/qml/QtQuick/Templates/qmldir                                                       "$TARGET/qml/QtQuick/Templates/"
cp -f $QT_DIR/qml/QtQuick/Templates/libqtquicktemplates2plugin.so                                "$TARGET/qml/QtQuick/Templates/"
cp -f $QT_DIR/qml/QtQuick/Dialogs/qmldir                                                         "$TARGET/qml/QtQuick/Dialogs/"
cp -f $QT_DIR/qml/QtQuick/Dialogs/libqtquickdialogsplugin.so                                     "$TARGET/qml/QtQuick/Dialogs/"
cp -f $QT_DIR/qml/QtQuick/Dialogs/quickimpl/qmldir                                               "$TARGET/qml/QtQuick/Dialogs/quickimpl/"
cp -f $QT_DIR/qml/QtQuick/Dialogs/quickimpl/qml/*.qml                                            "$TARGET/qml/QtQuick/Dialogs/quickimpl/qml/"
cp -f $QT_DIR/qml/QtQuick/Dialogs/quickimpl/qml/+Material/*.qml                                  "$TARGET/qml/QtQuick/Dialogs/quickimpl/qml/+Material/"
cp -f $QT_DIR/qml/QtQuick/Dialogs/quickimpl/libqtquickdialogs2quickimplplugin.so                 "$TARGET/qml/QtQuick/Dialogs/quickimpl/"

mkdir -p "$TARGET/plugins/iconengines/"
mkdir -p "$TARGET/plugins/imageformats/"
mkdir -p "$TARGET/plugins/platforms/"
mkdir -p "$TARGET/plugins/generic/"
mkdir -p "$TARGET/plugins/platforminputcontexts/"
mkdir -p "$TARGET/plugins/platformthemes/"
mkdir -p "$TARGET/plugins/egldeviceintegrations/"
mkdir -p "$TARGET/plugins/wayland-decoration-client/"
mkdir -p "$TARGET/plugins/wayland-graphics-integration-client/"
mkdir -p "$TARGET/plugins/wayland-shell-integration/"
mkdir -p "$TARGET/plugins/xcbglintegrations/"
cp -f $QT_DIR/plugins/iconengines/libqsvgicon.so                                                 "$TARGET/plugins/iconengines/"
cp -f $QT_DIR/plugins/imageformats/libqsvg.so                                                    "$TARGET/plugins/imageformats/"
cp -f $QT_DIR/plugins/platforms/*.so                                                             "$TARGET/plugins/platforms/"
cp -f $QT_DIR/plugins/generic/*.so                                                               "$TARGET/plugins/generic/"
cp -f $QT_DIR/plugins/platforminputcontexts/*.so                                                 "$TARGET/plugins/platforminputcontexts/"
cp -f $QT_DIR/plugins/platformthemes/*.so                                                        "$TARGET/plugins/platformthemes/"
cp -f $QT_DIR/plugins/egldeviceintegrations/*.so                                                 "$TARGET/plugins/egldeviceintegrations/"
cp -f $QT_DIR/plugins/wayland-decoration-client/*.so                                             "$TARGET/plugins/wayland-decoration-client/"
cp -f $QT_DIR/plugins/wayland-graphics-integration-client/*.so                                   "$TARGET/plugins/wayland-graphics-integration-client/"
cp -f $QT_DIR/plugins/wayland-shell-integration/*.so                                             "$TARGET/plugins/wayland-shell-integration/"
cp -f $QT_DIR/plugins/xcbglintegrations/*.so                                                     "$TARGET/plugins/xcbglintegrations/"

cp -f "$CARGO_TARGET/libmdk.so.0"                      "$TARGET/lib/"
#cp -f "$CARGO_TARGET/libffmpeg.so.5"                  "$TARGET/"

cp -f "$FFMPEG_DIR/lib/libavcodec.so.59"               "$TARGET/lib/"
cp -f "$FFMPEG_DIR/lib/libavfilter.so.8"               "$TARGET/lib/"
cp -f "$FFMPEG_DIR/lib/libavformat.so.59"              "$TARGET/lib/"
cp -f "$FFMPEG_DIR/lib/libavutil.so.57"                "$TARGET/lib/"
cp -f "$FFMPEG_DIR/lib/libswresample.so.4"             "$TARGET/lib/"
cp -f "$FFMPEG_DIR/lib/libswscale.so.6"                "$TARGET/lib/"
cp -f "$FFMPEG_DIR/lib/amd64/libavcodec.so.59"         "$TARGET/lib/"
cp -f "$FFMPEG_DIR/lib/amd64/libavfilter.so.8"         "$TARGET/lib/"
cp -f "$FFMPEG_DIR/lib/amd64/libavformat.so.59"        "$TARGET/lib/"
cp -f "$FFMPEG_DIR/lib/amd64/libavutil.so.57"          "$TARGET/lib/"
cp -f "$FFMPEG_DIR/lib/amd64/libswresample.so.4"       "$TARGET/lib/"
cp -f "$FFMPEG_DIR/lib/amd64/libswscale.so.6"          "$TARGET/lib/"

cp -f "$CARGO_TARGET/gyroflow"                         "$TARGET/"
strip "$TARGET/gyroflow"

cp -rf "$PROJECT_DIR/resources/camera_presets"         "$TARGET/"

pushd $TARGET/..
tar -czf Gyroflow-linux64.tar.gz --transform 's!linux64!Gyroflow!' linux64

# ---- Build AppImage ----
export APP_DIR=$TARGET/../AppDir
export APP_VERSION=1.0.0-rc4

rm -rf $APP_DIR
mkdir -p $APP_DIR/usr/share/icons
cp -f $PROJECT_DIR/_deployment/linux/gyroflow.png $APP_DIR/usr/share/icons/
cp -f $PROJECT_DIR/_deployment/linux/gyroflow.svg $APP_DIR/usr/share/icons/

cp -rf $TARGET/* $APP_DIR/
appimage-builder --recipe $PROJECT_DIR/_deployment/linux/AppImageBuilder.yml
chmod +x Gyroflow-${APP_VERSION}-x86_64.AppImage
mv Gyroflow-${APP_VERSION}-x86_64.AppImage Gyroflow-linux64.AppImage
# ---- Build AppImage ----

rm -rf $APP_DIR
rm -rf $TARGET

popd
