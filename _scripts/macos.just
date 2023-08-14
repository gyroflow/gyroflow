!include common.just

# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ Dependencies ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

[no-cd]
install-deps:
    #!/bin/bash
    brew install p7zip python pkg-config
    xcode-select --install

    mkdir -p {{ExtDir}}
    cd {{ExtDir}}

    # Install vcpkg
    git clone --depth 1 https://github.com/Microsoft/vcpkg.git
    ./vcpkg/bootstrap-vcpkg.sh -disableMetrics

    if [ ! -d "{{ExtDir}}/{{QtVersion}}" ]; then
        # Install Qt
        pip3 install -U pip aqtinstall
        python3 -m aqt install-qt mac desktop {{QtVersion}}
    fi

    if [ ! -f "{{OpenCVPath}}/../lib/libopencv_core4.a" ]; then
        echo "Installing OpenCV from vcpkg"
        # Install OpenCV
        ./vcpkg/vcpkg install "opencv4[core]:x64-osx-release"
        ./vcpkg/vcpkg install "opencv4[core]:arm64-osx"
        rm -rf ./vcpkg/buildtrees
        rm -rf ./vcpkg/downloads
    fi

    if [ ! -d "{{FFmpeg}}" ]; then
        echo "Downloading ffmpeg"

        # Download and extract ffmpeg
        curl -L https://sourceforge.net/projects/avbuild/files/macOS/ffmpeg-6.0-macOS-gpl-lite.tar.xz/download -o ffmpeg.tar.xz
        7z x -aoa ffmpeg.tar.xz
        tar -xf ffmpeg.tar
        rm ffmpeg.tar ffmpeg.tar.xz
        mkdir -p ffmpeg-x86_64/lib
        mkdir -p ffmpeg-arm64/lib
        cd ffmpeg-6.0-macOS-gpl-lite
        lipo lib/libavcodec.a    -thin x86_64 -output ../ffmpeg-x86_64/lib/libavcodec.a
        lipo lib/libavformat.a   -thin x86_64 -output ../ffmpeg-x86_64/lib/libavformat.a
        lipo lib/libavdevice.a   -thin x86_64 -output ../ffmpeg-x86_64/lib/libavdevice.a
        lipo lib/libavfilter.a   -thin x86_64 -output ../ffmpeg-x86_64/lib/libavfilter.a
        lipo lib/libavutil.a     -thin x86_64 -output ../ffmpeg-x86_64/lib/libavutil.a
        lipo lib/libswresample.a -thin x86_64 -output ../ffmpeg-x86_64/lib/libswresample.a
        lipo lib/libswscale.a    -thin x86_64 -output ../ffmpeg-x86_64/lib/libswscale.a
        lipo lib/libx264.a       -thin x86_64 -output ../ffmpeg-x86_64/lib/libx264.a
        lipo lib/libx265.a       -thin x86_64 -output ../ffmpeg-x86_64/lib/libx265.a

        lipo lib/libavcodec.a    -thin arm64 -output ../ffmpeg-arm64/lib/libavcodec.a
        lipo lib/libavformat.a   -thin arm64 -output ../ffmpeg-arm64/lib/libavformat.a
        lipo lib/libavdevice.a   -thin arm64 -output ../ffmpeg-arm64/lib/libavdevice.a
        lipo lib/libavfilter.a   -thin arm64 -output ../ffmpeg-arm64/lib/libavfilter.a
        lipo lib/libavutil.a     -thin arm64 -output ../ffmpeg-arm64/lib/libavutil.a
        lipo lib/libswresample.a -thin arm64 -output ../ffmpeg-arm64/lib/libswresample.a
        lipo lib/libswscale.a    -thin arm64 -output ../ffmpeg-arm64/lib/libswscale.a
        lipo lib/libx264.a       -thin arm64 -output ../ffmpeg-arm64/lib/libx264.a
        lipo lib/libx265.a       -thin arm64 -output ../ffmpeg-arm64/lib/libx265.a
        cp -R include ../ffmpeg-x86_64/include
        cp -R include ../ffmpeg-arm64/include
    fi

# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ Development ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

[no-cd]
run *param:
    cargo run --release -- {{param}}

[no-cd]
debug *param:
    cargo run -- {{param}}

# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ Deployment ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

[no-cd]
deploy *param:
    #!/bin/bash

    : "${PROJECT_DIR:=$(cd "{{ProjectDir}}"; pwd -P)}"
    : "${CARGO_TARGET:=$PROJECT_DIR/target/release}"
    : "${QT_DIR:=$PROJECT_DIR/ext/{{QtVersion}}/macos}"
    : "${OPENCV_DIR:=$PROJECT_DIR/ext/vcpkg/installed}"

    rm -rf "$PROJECT_DIR/_deployment/_binaries/mac"

    if [ "{{param}}" == "local" ]; then
        pushd $PROJECT_DIR
        cargo build --profile deploy
        popd
    fi

    if [ "{{param}}" == "universal" ]; then
        pushd $PROJECT_DIR

        export PATH="$PROJECT_DIR/ext/{{QtVersion}}/macos/bin:$PATH"
        export OPENCV_LINK_LIBS="opencv_core4,opencv_calib3d4,opencv_features2d4,opencv_imgproc4,opencv_video4,opencv_flann4,opencv_stitching4"

        #export DYLD_FALLBACK_LIBRARY_PATH="$(xcode-select --print-path)/usr/lib/"
        export DYLD_FALLBACK_LIBRARY_PATH="$(xcode-select --print-path)/Toolchains/XcodeDefault.xctoolchain/usr/lib/"
        #export LD_LIBRARY_PATH="$PROJECT_DIR/ext/{{QtVersion}}/macos/lib"
        export MACOSX_DEPLOYMENT_TARGET="10.11"

        export FFMPEG_DIR=$PROJECT_DIR/ext/ffmpeg-x86_64
        export OPENCV_LINK_PATHS=$OPENCV_DIR/x64-osx-release/lib
        export OPENCV_INCLUDE_PATHS=$OPENCV_DIR/x64-osx-release/include/
        rustup target add x86_64-apple-darwin
        cargo build --target x86_64-apple-darwin --profile deploy
        strip $PROJECT_DIR/target/x86_64-apple-darwin/deploy/gyroflow

        export OPENCV_LINK_LIBS="$OPENCV_LINK_LIBS,tegra_hal"
        export FFMPEG_DIR=$PROJECT_DIR/ext/ffmpeg-arm64
        export OPENCV_LINK_PATHS=$OPENCV_DIR/arm64-osx/lib,$OPENCV_DIR/arm64-osx/lib/manual-link/opencv4_thirdparty
        export OPENCV_INCLUDE_PATHS=$OPENCV_DIR/arm64-osx/include/
        export MACOSX_DEPLOYMENT_TARGET="11.0"
        rustup target add aarch64-apple-darwin
        cargo build --target aarch64-apple-darwin --profile deploy
        strip $PROJECT_DIR/target/aarch64-apple-darwin/deploy/gyroflow

        lipo $PROJECT_DIR/target/{x86_64,aarch64}-apple-darwin/deploy/gyroflow -create -output $PROJECT_DIR/target/deploy/gyroflow

        popd
    fi

    if [ "{{param}}" == "local" ] || [ "{{param}}" == "universal" ]; then
        mkdir -p "$PROJECT_DIR/_deployment/_binaries/mac"
        CARGO_TARGET="$PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/MacOS"
        cp -Rf "$PROJECT_DIR/_deployment/mac/Gyroflow.app"    "$PROJECT_DIR/_deployment/_binaries/mac/"
        strip  "$PROJECT_DIR/target/deploy/gyroflow"
        cp -f  "$PROJECT_DIR/target/deploy/gyroflow"          "$PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/MacOS/"
        cp -Rf "$PROJECT_DIR/target/Frameworks/mdk.framework" "$PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/Frameworks/"
        cp -Rf "$PROJECT_DIR/target/x86_64-apple-darwin/Frameworks/mdk.framework" "$PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/Frameworks/"
        cp -Rf "$PROJECT_DIR/resources/camera_presets"        "$PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/Resources/"
    fi

    cp -af "$QT_DIR/lib/QtCore.framework"                     "$CARGO_TARGET/../Frameworks/"
    cp -af "$QT_DIR/lib/QtDBus.framework"                     "$CARGO_TARGET/../Frameworks/"
    cp -af "$QT_DIR/lib/QtGui.framework"                      "$CARGO_TARGET/../Frameworks/"
    cp -af "$QT_DIR/lib/QtNetwork.framework"                  "$CARGO_TARGET/../Frameworks/"
    cp -af "$QT_DIR/lib/QtOpenGL.framework"                   "$CARGO_TARGET/../Frameworks/"
    cp -af "$QT_DIR/lib/QtQml.framework"                      "$CARGO_TARGET/../Frameworks/"
    cp -af "$QT_DIR/lib/QtQmlCore.framework"                  "$CARGO_TARGET/../Frameworks/"
    cp -af "$QT_DIR/lib/QtQmlModels.framework"                "$CARGO_TARGET/../Frameworks/"
    cp -af "$QT_DIR/lib/QtQmlWorkerScript.framework"          "$CARGO_TARGET/../Frameworks/"
    cp -af "$QT_DIR/lib/QtQuick.framework"                    "$CARGO_TARGET/../Frameworks/"
    cp -af "$QT_DIR/lib/QtQuickControls2.framework"           "$CARGO_TARGET/../Frameworks/"
    cp -af "$QT_DIR/lib/QtQuickControls2Impl.framework"       "$CARGO_TARGET/../Frameworks/"
    cp -af "$QT_DIR/lib/QtQuickDialogs2.framework"            "$CARGO_TARGET/../Frameworks/"
    cp -af "$QT_DIR/lib/QtQuickDialogs2QuickImpl.framework"   "$CARGO_TARGET/../Frameworks/"
    cp -af "$QT_DIR/lib/QtQuickDialogs2Utils.framework"       "$CARGO_TARGET/../Frameworks/"
    cp -af "$QT_DIR/lib/QtQuickTemplates2.framework"          "$CARGO_TARGET/../Frameworks/"
    cp -af "$QT_DIR/lib/QtQuickLayouts.framework"             "$CARGO_TARGET/../Frameworks/"
    cp -af "$QT_DIR/lib/QtSvg.framework"                      "$CARGO_TARGET/../Frameworks/"
    cp -af "$QT_DIR/lib/QtWidgets.framework"                  "$CARGO_TARGET/../Frameworks/"
    # Remove once moved to Qt 6.5.2
    cp -af "$QT_DIR/lib/QtLabsSettings.framework"             "$CARGO_TARGET/../Frameworks/"

    if [ "{{param}}" == "local" ] || [ "{{param}}" == "universal" ]; then
        CARGO_TARGET="$PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/Resources/qml"
    fi

    # Remove once moved to Qt 6.5.2
    mkdir -p "$CARGO_TARGET/Qt/labs/settings/"
    cp -f $QT_DIR/qml/Qt/labs/settings/qmldir                      "$CARGO_TARGET/Qt/labs/settings/"
    cp -f $QT_DIR/qml/Qt/labs/settings/libqmlsettingsplugin.dylib  "$CARGO_TARGET/Qt/labs/settings/"

    mkdir -p "$CARGO_TARGET/QtCore/"
    mkdir -p "$CARGO_TARGET/QtQml/Base/"
    mkdir -p "$CARGO_TARGET/QtQml/Models/"
    mkdir -p "$CARGO_TARGET/QtQml/WorkerScript/"
    mkdir -p "$CARGO_TARGET/QtQuick/Controls/impl/"
    mkdir -p "$CARGO_TARGET/QtQuick/Controls/macOS/"
    mkdir -p "$CARGO_TARGET/QtQuick/Controls/Basic/impl/"
    mkdir -p "$CARGO_TARGET/QtQuick/Controls/Material/impl/"
    mkdir -p "$CARGO_TARGET/QtQuick/Layouts/"
    mkdir -p "$CARGO_TARGET/QtQuick/Window/"
    mkdir -p "$CARGO_TARGET/QtQuick/Templates/"
    mkdir -p "$CARGO_TARGET/QtQuick/Dialogs/quickimpl/qml/+Material/"

    cp -f $QT_DIR/qml/QtCore/qmldir                                                                   "$CARGO_TARGET/QtCore/"
    cp -f $QT_DIR/qml/QtCore/libqtqmlcoreplugin.dylib                                                 "$CARGO_TARGET/QtCore/"

    cp -f $QT_DIR/qml/QtQml/qmldir                                                                    "$CARGO_TARGET/QtQml/"
    cp -f $QT_DIR/qml/QtQml/libqmlplugin.dylib                                                        "$CARGO_TARGET/QtQml/"
    cp -f $QT_DIR/qml/QtQml/libqmlmetaplugin.dylib                                                    "$CARGO_TARGET/QtQml/"
    cp -f $QT_DIR/qml/QtQml/Base/libqmlplugin.dylib                                                   "$CARGO_TARGET/QtQml/Base/"
    cp -f $QT_DIR/qml/QtQml/Base/qmldir                                                               "$CARGO_TARGET/QtQml/Base/"
    cp -f $QT_DIR/qml/QtQml/Models/libmodelsplugin.dylib                                              "$CARGO_TARGET/QtQml/Models/"
    cp -f $QT_DIR/qml/QtQml/Models/qmldir                                                             "$CARGO_TARGET/QtQml/Models/"
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

    if [ "{{param}}" == "local" ] || [ "{{param}}" == "universal" ]; then
        CARGO_TARGET="$PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/PlugIns"
    fi
    mkdir -p "$CARGO_TARGET/iconengines/"
    mkdir -p "$CARGO_TARGET/imageformats/"
    mkdir -p "$CARGO_TARGET/platforms/"
    cp -f $QT_DIR/plugins/iconengines/libqsvgicon.dylib                                               "$CARGO_TARGET/iconengines/"
    cp -f $QT_DIR/plugins/imageformats/libqsvg.dylib                                                  "$CARGO_TARGET/imageformats/"
    cp -f $QT_DIR/plugins/imageformats/libqjpeg.dylib                                                 "$CARGO_TARGET/imageformats/"
    cp -f $QT_DIR/plugins/platforms/libqcocoa.dylib                                                   "$CARGO_TARGET/platforms/"

    if [ "{{param}}" == "local" ] || [ "{{param}}" == "universal" ]; then
        xattr -c $PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/Info.plist
        xattr -c $PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/Resources/icon.icns
        rm -f $PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/MacOS/.empty
        rm -f $PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/PlugIns/.empty
        rm -f $PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/Frameworks/.empty
        rm -f $PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/Frameworks/mdk.framework/Versions/A/libass.9.dylib

        if [ "$SIGNING_FINGERPRINT" != "" ]; then

            # Certificate needs to be "Developer ID Application"

            OBJECTS=(
                "Frameworks/mdk.framework/Versions/A/libffmpeg.6.dylib"
                #"Frameworks/mdk.framework/Versions/A/libass.9.dylib"
                "Frameworks/mdk.framework/Versions/A/libmdk-braw.dylib"
                "Frameworks/mdk.framework/Versions/A/libmdk-r3d.dylib"
                "Frameworks/mdk.framework/Versions/A/mdk"
                "Frameworks/QtCore.framework/Versions/A/QtCore"
                "Frameworks/QtDBus.framework/Versions/A/QtDBus"
                "Frameworks/QtGui.framework/Versions/A/QtGui"
                "Frameworks/QtLabsSettings.framework/Versions/A/QtLabsSettings"
                "Frameworks/QtNetwork.framework/Versions/A/QtNetwork"
                "Frameworks/QtOpenGL.framework/Versions/A/QtOpenGL"
                "Frameworks/QtQml.framework/Versions/A/QtQml"
                "Frameworks/QtQmlCore.framework/Versions/A/QtQmlCore"
                "Frameworks/QtQmlModels.framework/Versions/A/QtQmlModels"
                "Frameworks/QtQmlWorkerScript.framework/Versions/A/QtQmlWorkerScript"
                "Frameworks/QtQuick.framework/Versions/A/QtQuick"
                "Frameworks/QtQuickControls2.framework/Versions/A/QtQuickControls2"
                "Frameworks/QtQuickControls2Impl.framework/Versions/A/QtQuickControls2Impl"
                "Frameworks/QtQuickDialogs2.framework/Versions/A/QtQuickDialogs2"
                "Frameworks/QtQuickDialogs2QuickImpl.framework/Versions/A/QtQuickDialogs2QuickImpl"
                "Frameworks/QtQuickDialogs2Utils.framework/Versions/A/QtQuickDialogs2Utils"
                "Frameworks/QtQuickTemplates2.framework/Versions/A/QtQuickTemplates2"
                "Frameworks/QtQuickLayouts.framework/Versions/A/QtQuickLayouts"
                "Frameworks/QtSvg.framework/Versions/A/QtSvg"
                "Frameworks/QtWidgets.framework/Versions/A/QtWidgets"
                "PlugIns/iconengines/libqsvgicon.dylib"
                "PlugIns/imageformats/libqsvg.dylib"
                "PlugIns/imageformats/libqjpeg.dylib"
                "PlugIns/platforms/libqcocoa.dylib"
                "Resources/qml/Qt/labs/settings/libqmlsettingsplugin.dylib"
                "Resources/qml/QtQml/libqmlplugin.dylib"
                "Resources/qml/QtQml/Models/libmodelsplugin.dylib"
                "Resources/qml/QtQml/WorkerScript/libworkerscriptplugin.dylib"
                "Resources/qml/QtCore/libqtqmlcoreplugin.dylib"
                "Resources/qml/QtQuick/libqtquick2plugin.dylib"
                "Resources/qml/QtQuick/Controls/libqtquickcontrols2plugin.dylib"
                "Resources/qml/QtQuick/Controls/Basic/libqtquickcontrols2basicstyleplugin.dylib"
                "Resources/qml/QtQuick/Controls/Basic/impl/libqtquickcontrols2basicstyleimplplugin.dylib"
                "Resources/qml/QtQuick/Controls/impl/libqtquickcontrols2implplugin.dylib"
                "Resources/qml/QtQuick/Controls/macOS/libqtquickcontrols2macosstyleplugin.dylib"
                "Resources/qml/QtQuick/Controls/Material/libqtquickcontrols2materialstyleplugin.dylib"
                "Resources/qml/QtQuick/Controls/Material/impl/libqtquickcontrols2materialstyleimplplugin.dylib"
                "Resources/qml/QtQuick/Dialogs/libqtquickdialogsplugin.dylib"
                "Resources/qml/QtQuick/Dialogs/quickimpl/libqtquickdialogs2quickimplplugin.dylib"
                "Resources/qml/QtQuick/Layouts/libqquicklayoutsplugin.dylib"
                "Resources/qml/QtQuick/Templates/libqtquicktemplates2plugin.dylib"
                "Resources/qml/QtQuick/Window/libquickwindowplugin.dylib"
                "MacOS/gyroflow"
            )
            for i in "${OBJECTS[@]}"
            do
                codesign -vvvv --strict --options=runtime --timestamp --force -s $SIGNING_FINGERPRINT $PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/$i
            done

            codesign -vvvv --strict --options=runtime --timestamp --force -s $SIGNING_FINGERPRINT $PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app

            codesign -vvvv --deep --verify $PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app
        fi

        ln -sf /Applications "$PROJECT_DIR/_deployment/_binaries/mac/Applications"
        sudo mdutil -a -i off
        hdiutil create "$PROJECT_DIR/_deployment/_binaries/Gyroflow-mac-universal.dmg" -volname "Gyroflow v{{ProjectVersion}}" -fs HFS+ -srcfolder "$PROJECT_DIR/_deployment/_binaries/mac/" -ov -format UDZO -imagekey zlib-level=9

        if [ "$SIGNING_FINGERPRINT" != "" ]; then
            codesign -vvvv --strict --options=runtime --timestamp --force -s $SIGNING_FINGERPRINT "$PROJECT_DIR/_deployment/_binaries/Gyroflow-mac-universal.dmg"
            codesign -vvvv --deep --verify "$PROJECT_DIR/_deployment/_binaries/Gyroflow-mac-universal.dmg"
        fi
    fi
