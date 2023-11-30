!include common.just

TeamID                        := "RR3ZC2L4DF"
BundleIdentifier              := "xyz.gyroflow"
DistributionCertificateSHA    := "7F82D6013B8F6D45F88FE547438744BD64750C5B"
DistributionMacInstallerSHA   := "DC70831576E7B2345E78A516BEA2FC34D18F42F5"
DistributionProvisioning      := ProjectDir / "_deployment/mac/Gyroflow_mac_app_store.provisionprofile"

# For local sandbox testing:
# DistributionCertificateSHA    := "D125BE70B690030172EB74A828C10C1AE77FDE9A"
# DistributionProvisioning      := ProjectDir / "_deployment/mac/Gyroflow_mac_developer_id.provisionprofile"

AppBundle := ProjectDir / "_deployment/_binaries/mac/Gyroflow.app"

# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ Dependencies ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

[no-cd]
install-deps:
    #!/bin/bash
    set -e

    brew install p7zip python pkg-config
    xcode-select --install || true

    mkdir -p {{ExtDir}}
    cd {{ExtDir}}

    # Install vcpkg
    git clone --depth 1 https://github.com/Microsoft/vcpkg.git || true
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
        curl -L https://sourceforge.net/projects/avbuild/files/macOS/ffmpeg-6.1-macOS-gpl-lite.tar.xz/download -o ffmpeg.tar.xz
        7z x -aoa ffmpeg.tar.xz
        tar -xf ffmpeg.tar
        rm ffmpeg.tar ffmpeg.tar.xz
        mkdir -p ffmpeg-x86_64/lib
        mkdir -p ffmpeg-arm64/lib
        cd ffmpeg-6.1-macOS-gpl-lite
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
    set -e

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
        #strip $PROJECT_DIR/target/x86_64-apple-darwin/deploy/gyroflow

        export OPENCV_LINK_LIBS="$OPENCV_LINK_LIBS,tegra_hal"
        export FFMPEG_DIR=$PROJECT_DIR/ext/ffmpeg-arm64
        export OPENCV_LINK_PATHS=$OPENCV_DIR/arm64-osx/lib,$OPENCV_DIR/arm64-osx/lib/manual-link/opencv4_thirdparty
        export OPENCV_INCLUDE_PATHS=$OPENCV_DIR/arm64-osx/include/
        export MACOSX_DEPLOYMENT_TARGET="11.0"
        rustup target add aarch64-apple-darwin
        cargo build --target aarch64-apple-darwin --profile deploy
        #strip $PROJECT_DIR/target/aarch64-apple-darwin/deploy/gyroflow

        lipo $PROJECT_DIR/target/{x86_64,aarch64}-apple-darwin/deploy/gyroflow -create -output $PROJECT_DIR/target/deploy/gyroflow

        popd
    fi

    ICON=$PROJECT_DIR/resources/icon_1024_mac.png
    ICON_OUT=$PROJECT_DIR/_deployment/mac/Gyroflow.app/Contents/Resources/Images.xcassets/AppIcon.appiconset
    sips -z 16 16      $ICON --out $ICON_OUT/icon_16x16.png
    sips -z 32 32      $ICON --out $ICON_OUT/icon_16x16@2x.png
    sips -z 32 32      $ICON --out $ICON_OUT/icon_32x32.png
    sips -z 64 64      $ICON --out $ICON_OUT/icon_32x32@2x.png
    sips -z 128 128    $ICON --out $ICON_OUT/icon_128x128.png
    sips -z 256 256    $ICON --out $ICON_OUT/icon_128x128@2x.png
    sips -z 256 256    $ICON --out $ICON_OUT/icon_256x256.png
    sips -z 512 512    $ICON --out $ICON_OUT/icon_256x256@2x.png
    sips -z 512 512    $ICON --out $ICON_OUT/icon_512x512.png
    sips -z 1024 1024  $ICON --out $ICON_OUT/icon_512x512@2x.png
    mkdir -p $ICON_OUT/../AppIcon.iconset/
    cp -f $ICON_OUT/*.png $ICON_OUT/../AppIcon.iconset/
    iconutil -c icns $ICON_OUT/../AppIcon.iconset --output $PROJECT_DIR/_deployment/mac/Gyroflow.app/Contents/Resources/AppIcon.icns
    rm -rf $ICON_OUT/../AppIcon.iconset

    if [ "{{param}}" == "local" ] || [ "{{param}}" == "universal" ]; then
        mkdir -p "$PROJECT_DIR/_deployment/_binaries/mac"
        CARGO_TARGET="$PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/MacOS"
        cp -Rf "$PROJECT_DIR/_deployment/mac/Gyroflow.app"    "$PROJECT_DIR/_deployment/_binaries/mac/"
        cp -f  "$PROJECT_DIR/target/deploy/gyroflow"          "$PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/MacOS/"
        strip "$PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/MacOS/gyroflow"
        cp -Rf "$PROJECT_DIR/target/Frameworks/mdk.framework" "$PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/Frameworks/" || true
        cp -Rf "$PROJECT_DIR/target/x86_64-apple-darwin/Frameworks/mdk.framework" "$PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/Frameworks/"
        cp -Rf "$PROJECT_DIR/resources/camera_presets"        "$PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/Resources/"
        rm -rf "$PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/Resources/camera_presets/.git"
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
    cp -af "$QT_DIR/lib/QtQuickEffects.framework"             "$CARGO_TARGET/../Frameworks/" || true
    cp -af "$QT_DIR/lib/QtQuickTemplates2.framework"          "$CARGO_TARGET/../Frameworks/"
    cp -af "$QT_DIR/lib/QtQuickLayouts.framework"             "$CARGO_TARGET/../Frameworks/"
    cp -af "$QT_DIR/lib/QtSvg.framework"                      "$CARGO_TARGET/../Frameworks/"
    cp -af "$QT_DIR/lib/QtWidgets.framework"                  "$CARGO_TARGET/../Frameworks/"
    # Remove once moved to Qt 6.6.1
    cp -af "$QT_DIR/lib/QtLabsSettings.framework"             "$CARGO_TARGET/../Frameworks/"

    if [ "{{param}}" == "local" ] || [ "{{param}}" == "universal" ]; then
        CARGO_TARGET="$PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/Resources/qml"
    fi

    # Remove once moved to Qt 6.6.1
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
    mkdir -p "$CARGO_TARGET/QtQuick/Effects/"
    mkdir -p "$CARGO_TARGET/QtQuick/Window/"
    mkdir -p "$CARGO_TARGET/QtQuick/Templates/"
    mkdir -p "$CARGO_TARGET/QtQuick/Dialogs/quickimpl/qml/+Material/"

    cp -f $QT_DIR/qml/QtCore/qmldir                                                                   "$CARGO_TARGET/QtCore/"
    cp -f $QT_DIR/qml/QtCore/libqtqmlcoreplugin.dylib                                                 "$CARGO_TARGET/QtCore/"

    cp -f $QT_DIR/qml/QtQml/qmldir                                                                    "$CARGO_TARGET/QtQml/"
    cp -f $QT_DIR/qml/QtQml/libqmlplugin.dylib                                                        "$CARGO_TARGET/QtQml/" || true
    cp -f $QT_DIR/qml/QtQml/libqmlmetaplugin.dylib                                                    "$CARGO_TARGET/QtQml/" || true
    cp -f $QT_DIR/qml/QtQml/Base/libqmlplugin.dylib                                                   "$CARGO_TARGET/QtQml/Base/" || true
    cp -f $QT_DIR/qml/QtQml/Base/qmldir                                                               "$CARGO_TARGET/QtQml/Base/" || true
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
    cp -f $QT_DIR/qml/QtQuick/Effects/libeffectsplugin.dylib                                          "$CARGO_TARGET/QtQuick/Effects/" || true
    cp -f $QT_DIR/qml/QtQuick/Effects/qmldir                                                          "$CARGO_TARGET/QtQuick/Effects/" || true

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
        xattr -c $PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/Resources/AppIcon.icns
        rm -f $PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/MacOS/.empty
        rm -f $PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/PlugIns/.empty
        rm -f $PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/Frameworks/.empty
        rm -f $PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/Frameworks/mdk.framework/Versions/A/libass.dylib

        if [ "$SIGNING_FINGERPRINT" != "" ]; then
            # Certificate needs to be "Developer ID Application"

            find $PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents \( -name "*.dylib" -or -path "*/Versions/A/*" \) -and ! \( \
                -path "*/Headers/*" -or -path "*/A/Resources/*" -or -name "CodeResources" -or -name "module.modulemap" \) -type f | sort | xargs -r \
                codesign -vvvv --strict --options runtime --timestamp --force -s $SIGNING_FINGERPRINT

            codesign -vvvv --strict --options runtime --timestamp --force -s $SIGNING_FINGERPRINT $PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app/Contents/MacOS/gyroflow

            codesign -vvvv --strict --options runtime --timestamp --force -s $SIGNING_FINGERPRINT $PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app

            codesign -vvvv --deep --verify $PROJECT_DIR/_deployment/_binaries/mac/Gyroflow.app
        fi

        if [ -z "$DONT_BUILD_DMG" ]; then
            ln -sf /Applications "$PROJECT_DIR/_deployment/_binaries/mac/Applications"
            sudo mdutil -a -i off
            hdiutil create "$PROJECT_DIR/_deployment/_binaries/Gyroflow-mac-universal.dmg" -volname "Gyroflow v{{ProjectVersion}}" -fs HFS+ -srcfolder "$PROJECT_DIR/_deployment/_binaries/mac/" -ov -format UDZO -imagekey zlib-level=9

            if [ "$SIGNING_FINGERPRINT" != "" ]; then
                codesign -vvvv --strict --options runtime --timestamp --force -s $SIGNING_FINGERPRINT "$PROJECT_DIR/_deployment/_binaries/Gyroflow-mac-universal.dmg"
                codesign -vvvv --deep --verify "$PROJECT_DIR/_deployment/_binaries/Gyroflow-mac-universal.dmg"
            fi
        fi
    fi

[no-cd]
bundle *param:
    #!/bin/bash
    set -e

    : "${PROJECT_DIR:=$(cd "{{ProjectDir}}"; pwd -P)}"
    export PATH="/usr/libexec/:$PATH"

    cp -f "$PROJECT_DIR/_deployment/mac/Gyroflow.app/Contents/Info.plist" {{AppBundle}}/Contents/Info.plist
    cp -f "$PROJECT_DIR/target/deploy/gyroflow" "{{AppBundle}}/Contents/MacOS/"

    if ! [ -z "$GITHUB_RUN_NUMBER" ]; then
        PlistBuddy -x -c "Set :CFBundleVersion \"`echo "{{ProjectVersion}}" | sed -E 's/[^0-9\.]//' | sed -E 's/\.([0-9]+)$/\1/'`.$GITHUB_RUN_NUMBER\"" {{AppBundle}}/Contents/Info.plist
    fi
    PATCH_NUM="0"
    if [ "$IS_MANUAL_DISPATCH" == "true" ]; then
        PATCH_NUM="1"
    fi
    PlistBuddy -x -c "Set :CFBundleShortVersionString \"`echo "{{ProjectVersion}}" | sed -E 's/[^0-9\.]//' | sed -E 's/\.([0-9]+)$/\1/'`.$PATCH_NUM\"" {{AppBundle}}/Contents/Info.plist

    DEFAULT_SDK_VERSION=`xcrun --sdk macosx --show-sdk-version`
    PlistBuddy -x -c "Add :BuildMachineOSBuild string `sw_vers -buildVersion`" {{AppBundle}}/Contents/Info.plist
    PlistBuddy -x -c "Add :DTCompiler string `defaults read $(xcode-select -p)/../info DTCompiler`" {{AppBundle}}/Contents/Info.plist
    PlistBuddy -x -c "Add :DTPlatformBuild string `xcrun --sdk macosx --show-sdk-build-version`" {{AppBundle}}/Contents/Info.plist
    PlistBuddy -x -c "Add :DTPlatformName string macosx" {{AppBundle}}/Contents/Info.plist
    PlistBuddy -x -c "Add :DTPlatformVersion string $DEFAULT_SDK_VERSION" {{AppBundle}}/Contents/Info.plist
    PlistBuddy -x -c "Add :DTSDKBuild string `xcrun --sdk macosx --show-sdk-build-version`" {{AppBundle}}/Contents/Info.plist
    PlistBuddy -x -c "Add :DTSDKName string macosx$DEFAULT_SDK_VERSION" {{AppBundle}}/Contents/Info.plist
    PlistBuddy -x -c "Add :DTXcode string `defaults read $(xcode-select -p)/../info DTXcode`" {{AppBundle}}/Contents/Info.plist
    PlistBuddy -x -c "Add :DTXcodeBuild string `xcodebuild -version | grep 'Build version' | sed 's/^Build version //'`" {{AppBundle}}/Contents/Info.plist

    # Bundle BRAW and RED SDK
    FrameworksDir={{AppBundle}}/Contents/Frameworks
    if [ ! -f "$FrameworksDir/REDR3D.dylib" ]; then
        curl -L https://api.gyroflow.xyz/sdk/RED_SDK_MacOS_8.4.0.tar.gz -o tmp.tar.gz
        7z x -aoa tmp.tar.gz
        7z x -o"$FrameworksDir/" tmp.tar
        rm tmp.tar tmp.tar.gz
    fi
    if [ ! -f "$FrameworksDir/BlackmagicRawAPI.framework/Versions/A/BlackmagicRawAPI" ]; then
        curl -L https://api.gyroflow.xyz/sdk/Blackmagic_RAW_SDK_MacOS_3.4.tar.gz -o tmp.tar.gz
        7z x -aoa tmp.tar.gz
        7z x -o"$FrameworksDir/" tmp.tar
        rm tmp.tar tmp.tar.gz
    fi
    chmod -R 0755 $FrameworksDir/*

    # Generate entitlements
    rm -f {{AppBundle}}/../entitlements.xcent
    PlistBuddy -x -c "Add :com.apple.security.app-sandbox bool true" {{AppBundle}}/../entitlements.xcent
    PlistBuddy -x -c "Add :com.apple.security.network.client bool true" {{AppBundle}}/../entitlements.xcent
    PlistBuddy -x -c "Add :com.apple.security.files.user-selected.read-write bool true" {{AppBundle}}/../entitlements.xcent
    PlistBuddy -x -c "Add :com.apple.security.files.bookmarks.app-scope bool true" {{AppBundle}}/../entitlements.xcent
    PlistBuddy -x -c "Add :com.apple.security.files.bookmarks.document-scope bool true" {{AppBundle}}/../entitlements.xcent
    PlistBuddy -x -c "Add :com.apple.security.get-task-allow bool true" {{AppBundle}}/../entitlements.xcent

    actool {{ProjectDir}}/_deployment/ios/Resources/Images.xcassets --compile {{AppBundle}}/Contents --platform macosx --minimum-deployment-target 10.14 --app-icon AppIcon --output-partial-info-plist {{AppBundle}}/../AppIcon.plist

    dsymutil {{AppBundle}}/Contents/MacOS/gyroflow -o {{AppBundle}}/../Gyroflow.app.dSYM
    mkdir -p {{AppBundle}}/../Symbols
    xcrun symbols -noTextInSOD -noDaemon -arch all -symbolsPackageDir {{AppBundle}}/../Symbols {{AppBundle}}/../Gyroflow.app.dSYM

    if [ -z "$SIGN_KEY" ] || [ -z "$PKG_SIGN_KEY" ] || [ -z "$PROVISIONING_PROFILE" ]; then
        SIGN_KEY="{{DistributionCertificateSHA}}"
        PKG_SIGN_KEY="{{DistributionMacInstallerSHA}}"
        PROVISIONING_PROFILE="{{DistributionProvisioning}}"
    fi

    cp -f "$PROVISIONING_PROFILE" {{AppBundle}}/Contents/embedded.provisionprofile

    find {{AppBundle}}/Contents \( -name "*.dylib" -or -path "*/Versions/A/*" \) -and ! \( \
        -path "*/Headers/*" -or -path "*/A/Resources/*" -or -name "CodeResources" -or -name "module.modulemap" \) -type f | sort | xargs -r \
        codesign -vvvv --strict --options runtime --timestamp --force -s $SIGN_KEY --entitlements {{AppBundle}}/../entitlements.xcent

    codesign -vvvv --deep --strict --options runtime --timestamp --force -s $SIGN_KEY --entitlements {{AppBundle}}/../entitlements.xcent {{AppBundle}}/Contents/MacOS/gyroflow
    codesign -vvvv --deep --strict --options runtime --timestamp --force -s $SIGN_KEY --entitlements {{AppBundle}}/../entitlements.xcent {{AppBundle}}

    PlistBuddy -x -c "Add :com.apple.application-identifier string {{TeamID}}.{{BundleIdentifier}}" {{AppBundle}}/../entitlements.xcent
    PlistBuddy -x -c "Add :com.apple.developer.team-identifier string {{TeamID}}" {{AppBundle}}/../entitlements.xcent
    codesign -vvvv --strict --options runtime --timestamp --force -s $SIGN_KEY --entitlements {{AppBundle}}/../entitlements.xcent {{AppBundle}}

    codesign -vvvv --deep --verify {{AppBundle}}

    xcrun productbuild --symbolication {{AppBundle}}/../Symbols --component {{AppBundle}} "/Applications" --sign $PKG_SIGN_KEY "{{AppBundle}}/../../Gyroflow.pkg"
