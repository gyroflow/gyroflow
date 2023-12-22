!include common.just

# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ Dependencies ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

[no-cd]
install-deps:
    #!/bin/bash
    set -e

    sudo apt-get install -y p7zip-full python3-pip clang libclang-dev bison pkg-config gperf curl unzip zip git
    sudo apt-get install -y libc++-dev libva-dev libvdpau-dev libvdpau1 mesa-va-drivers ocl-icd-opencl-dev opencl-headers
    sudo apt-get install -y libpulse-dev libasound-dev libxkbcommon-dev
    sudo apt-get install -y libfontconfig1 libfreetype6

    mkdir -p {{ExtDir}}

    if [ ! -f "{{OpenCVPath}}/../lib/libopencv_core4.a" ]; then
        echo "Installing OpenCV from vcpkg"
        # OpenCV dependencies
        sudo apt-get install -y libx11-dev libxft-dev libxext-dev autoconf libtool libglfw3 libgles2-mesa-dev libxrandr-dev libxi-dev libxcursor-dev libxdamage-dev libxinerama-dev libxxf86vm-dev libdbus-1-dev libxtst-dev libdbus-1-dev libxi-dev libxtst-dev libsystemd-dev

        # Install vcpkg
        git clone --depth 1 https://github.com/Microsoft/vcpkg.git {{ExtDir}}/vcpkg || true

        {{ExtDir}}/vcpkg/bootstrap-vcpkg.sh -disableMetrics

        # Install OpenCV
        {{ExtDir}}/vcpkg/vcpkg install "opencv4[core]:{{VcpkgArch}}"
        rm -rf {{ExtDir}}/vcpkg/buildtrees
        rm -rf {{ExtDir}}/vcpkg/downloads
    fi

    if [[ ! -d "${LIBCLANG_PATH}" ]]; then
        sudo apt-get install -y libclang-13-dev
    fi

    pushd {{ExtDir}}

    if [ ! -d "{{ExtDir}}/{{QtVersion}}" ]; then
        echo "Installing Qt {{QtVersion}}"
        # Install Qt
        pip3 install -U pip
        pip3 install -U aqtinstall
        python3 -m aqt install-qt linux desktop {{QtVersion}}

        # For VMware: sudo apt install libpocl2
    fi

    if [ "$IN_DOCKER" == "1" ]; then
        echo "Installing AppImage builder"

        sudo apt-get install -y debian-keyring debian-archive-keyring
        sudo apt-key adv --refresh-keys --keyserver keyserver.ubuntu.com
        sudo apt-get install -y python3-setuptools patchelf desktop-file-utils libgdk-pixbuf2.0-dev fakeroot strace fuse gtk-update-icon-cache
        sudo curl -L https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage -o /opt/appimagetool

        # workaround AppImage issues with Docker
        pushd /opt/ ; sudo chmod +x appimagetool; sed -i 's|AI\x02|\x00\x00\x00|' appimagetool; sudo ./appimagetool --appimage-extract ; popd
        sudo mv /opt/squashfs-root /opt/appimagetool.AppDir
        sudo ln -s /opt/appimagetool.AppDir/AppRun /usr/local/bin/appimagetool

        sudo pip3 install -U pip
        sudo pip3 install appimage-builder==0.9.2
    fi

    if [ ! -d "{{FFmpeg}}" ]; then
        echo "Downloading ffmpeg"

        # Download and extract ffmpeg
        curl -L https://sourceforge.net/projects/avbuild/files/linux/{{FFmpeg}}.tar.xz/download -o ffmpeg.tar.xz
        7z x -aoa ffmpeg.tar.xz
        tar -xf ffmpeg.tar
        rm ffmpeg.tar ffmpeg.tar.xz
    fi

    popd

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
    mkdir -p "{{ProjectDir}}/target/deploy" || true

    : "${PROJECT_DIR:=$(readlink -f {{ProjectDir}})}"
    : "${QT_DIR:=$(readlink -f {{QtPath}}/..)}"
    : "${CARGO_TARGET:=$(readlink -f {{ProjectDir}}/target/deploy)}"
    : "${FFMPEG_DIR:=$(readlink -f {{FFMPEG_DIR}})}"

    echo "Cargo target dir: $CARGO_TARGET"

    if [ "{{param}}" == "docker" ]; then
        sudo docker run -v $PROJECT_DIR:$PROJECT_DIR -v $HOME/.cargo:/root/.cargo debian:10 bash -c "
            set -e
            apt update
            echo 'debconf debconf/frontend select Noninteractive' | debconf-set-selections
            apt install -y sudo dialog apt-utils curl clang
            export RUNLEVEL=1
            export GITHUB_RUN_NUMBER=$GITHUB_RUN_NUMBER
            export VULKAN_SDK=$VULKAN_SDK
            export CARGO_TARGET=$CARGO_TARGET
            export IN_DOCKER=1
            curl https://sh.rustup.rs -sSf | sh -s -- -y --profile minimal
            source \$HOME/.cargo/env
            cargo install --version '>=1.13' just
            cd $PROJECT_DIR
            just install-deps
            just deploy
        "
        stat $HOME/.cargo
        stat $PROJECT_DIR/Cargo.toml
        sudo chown -R $(stat -c "%U:%G" $PROJECT_DIR/Cargo.toml) $HOME/.cargo
        sudo chown -R $(stat -c "%U:%G" $PROJECT_DIR/Cargo.toml) $PROJECT_DIR
        exit;
    fi

    cargo build --profile deploy

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
    cp -f "$QT_DIR/lib/libQt6LabsFolderListModel.so.6"     "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6LabsQmlModels.so.6"           "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6QuickControls2.so.6"          "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6QuickControls2Impl.so.6"      "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6QuickTemplates2.so.6"         "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6QuickDialogs2.so.6"           "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6QuickDialogs2QuickImpl.so.6"  "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6QuickDialogs2Utils.so.6"      "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6QuickEffects.so.6"            "$TARGET/lib/" || true
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

    # Remove once moved to Qt 6.6.1
    cp -f "$QT_DIR/lib/libQt6LabsSettings.so.6"                "$TARGET/lib/"
    mkdir -p "$TARGET/qml/Qt/labs/settings/"
    cp -f $QT_DIR/qml/Qt/labs/settings/qmldir                  "$TARGET/qml/Qt/labs/settings/"
    cp -f $QT_DIR/qml/Qt/labs/settings/libqmlsettingsplugin.so "$TARGET/qml/Qt/labs/settings/"

    mkdir -p "$TARGET/qml/Qt/labs/folderlistmodel/"
    mkdir -p "$TARGET/qml/QtCore/"
    mkdir -p "$TARGET/qml/QtQml/Base/"
    mkdir -p "$TARGET/qml/QtQml/Models/"
    mkdir -p "$TARGET/qml/QtQml/WorkerScript/"
    mkdir -p "$TARGET/qml/QtQuick/Controls/impl/"
    mkdir -p "$TARGET/qml/QtQuick/Controls/Basic/impl/"
    mkdir -p "$TARGET/qml/QtQuick/Controls/Material/impl/"
    mkdir -p "$TARGET/qml/QtQuick/Layouts/"
    mkdir -p "$TARGET/qml/QtQuick/Effects/"
    mkdir -p "$TARGET/qml/QtQuick/Window/"
    mkdir -p "$TARGET/qml/QtQuick/Templates/"
    mkdir -p "$TARGET/qml/QtQuick/Dialogs/quickimpl/qml/+Material/"

    cp -f $QT_DIR/qml/QtCore/qmldir                                                                  "$TARGET/qml/QtCore/"
    cp -f $QT_DIR/qml/QtCore/libqtqmlcoreplugin.so                                                   "$TARGET/qml/QtCore/"
    cp -f $QT_DIR/qml/Qt/labs/folderlistmodel/qmldir                                                 "$TARGET/qml/Qt/labs/folderlistmodel/"
    cp -f $QT_DIR/qml/Qt/labs/folderlistmodel/libqmlfolderlistmodelplugin.so                         "$TARGET/qml/Qt/labs/folderlistmodel/"
    cp -f $QT_DIR/qml/QtQml/qmldir                                                                   "$TARGET/qml/QtQml/"
    cp -f $QT_DIR/qml/QtQml/libqmlmetaplugin.so                                                      "$TARGET/qml/QtQml/" || true
    cp -f $QT_DIR/qml/QtQml/libqmlplugin.so                                                          "$TARGET/qml/QtQml/" || true
    cp -f $QT_DIR/qml/QtQml/Base/libqmlplugin.so                                                     "$TARGET/qml/QtQml/Base/" || true
    cp -f $QT_DIR/qml/QtQml/Base/qmldir                                                              "$TARGET/qml/QtQml/Base/" || true
    cp -f $QT_DIR/qml/QtQml/Models/libmodelsplugin.so                                                "$TARGET/qml/QtQml/Models/"
    cp -f $QT_DIR/qml/QtQml/Models/qmldir                                                            "$TARGET/qml/QtQml/Models/"
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
    cp -f $QT_DIR/qml/QtQuick/Effects/libeffectsplugin.so                                            "$TARGET/qml/QtQuick/Effects/" || true
    cp -f $QT_DIR/qml/QtQuick/Effects/qmldir                                                         "$TARGET/qml/QtQuick/Effects/" || true

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
    cp -f $QT_DIR/plugins/imageformats/libqjpeg.so                                                   "$TARGET/plugins/imageformats/"
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
    cp -f "$CARGO_TARGET/libmdk-braw.so"                   "$TARGET/lib/"
    cp -f "$CARGO_TARGET/libmdk-r3d.so"                    "$TARGET/lib/"
    #cp -f "$CARGO_TARGET/libffmpeg.so.6"                  "$TARGET/"

    cp -f "$FFMPEG_DIR/lib/libavcodec.so.60"               "$TARGET/lib/"
    cp -f "$FFMPEG_DIR/lib/libavfilter.so.9"               "$TARGET/lib/"
    cp -f "$FFMPEG_DIR/lib/libavformat.so.60"              "$TARGET/lib/"
    cp -f "$FFMPEG_DIR/lib/libavutil.so.58"                "$TARGET/lib/"
    cp -f "$FFMPEG_DIR/lib/libswresample.so.4"             "$TARGET/lib/"
    cp -f "$FFMPEG_DIR/lib/libswscale.so.7"                "$TARGET/lib/"
    cp -f "$FFMPEG_DIR/lib/amd64/libavcodec.so.60"         "$TARGET/lib/" || true
    cp -f "$FFMPEG_DIR/lib/amd64/libavfilter.so.9"         "$TARGET/lib/" || true
    cp -f "$FFMPEG_DIR/lib/amd64/libavformat.so.60"        "$TARGET/lib/" || true
    cp -f "$FFMPEG_DIR/lib/amd64/libavutil.so.58"          "$TARGET/lib/" || true
    cp -f "$FFMPEG_DIR/lib/amd64/libswresample.so.4"       "$TARGET/lib/" || true
    cp -f "$FFMPEG_DIR/lib/amd64/libswscale.so.7"          "$TARGET/lib/" || true

    cp -f "$CARGO_TARGET/gyroflow"                         "$TARGET/"
    strip "$TARGET/gyroflow"

    cp -rf "$PROJECT_DIR/resources/camera_presets"         "$TARGET/"
    rm -rf "$TARGET/camera_presets/.git"

    pushd $TARGET/..
    tar -czf Gyroflow-linux64.tar.gz --transform 's!linux64!Gyroflow!' linux64

    # ---- Build AppImage ----
    export APP_DIR=$TARGET/../AppDir
    export APP_VERSION={{ProjectVersion}}

    rm -rf $APP_DIR
    mkdir -p $APP_DIR/usr/share/icons
    cp -f $PROJECT_DIR/_deployment/linux/gyroflow.png $APP_DIR/usr/share/icons/
    cp -f $PROJECT_DIR/_deployment/linux/gyroflow.svg $APP_DIR/usr/share/icons/

    cp -rf $TARGET/* $APP_DIR/
    appimage-builder --recipe $PROJECT_DIR/_deployment/linux/AppImageBuilder.yml
    chmod +x Gyroflow-${APP_VERSION}-x86_64.AppImage

    # Try to fix AppImage permissions
    ./Gyroflow-${APP_VERSION}-x86_64.AppImage --appimage-extract
    rm -f Gyroflow-${APP_VERSION}-x86_64.AppImage
    sudo chmod 755 squashfs-root
    find ./squashfs-root -type d -exec sudo chmod 755 {} \;
    mv squashfs-root Gyroflow.AppDir
    ARCH=x86_64 appimagetool ./Gyroflow.AppDir
    rm -rf Gyroflow.AppDir

    mv Gyroflow-x86_64.AppImage Gyroflow-linux64.AppImage
    # ---- Build AppImage ----

    rm -rf $APP_DIR
    rm -rf $TARGET

    popd
