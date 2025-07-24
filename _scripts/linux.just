import 'common.just'

AptGetExists := `which apt-get > /dev/null && echo true || echo false`
QtLinuxArch      := if Arch == "aarch64" { "linux_arm64" } else { "linux" }
FFmpegLinuxArch  := if Arch == "aarch64" { "arm64"       } else { "amd64" }
ArchName         := if Arch == "aarch64" { "arm64"       } else { "x86_64" }
PackageSuffix    := if Arch == "aarch64" { "linux-arm64" } else { "linux64" }

# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ Dependencies ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

[no-cd]
install-deps:
    #!/bin/bash
    set -e

    echo "Installing dependencies $LIBCLANG_PATH {{AptGetExists}}"
    if [ "{{AptGetExists}}" == "true" ]; then
        sudo apt-get install -y p7zip-full python3-pip python3-venv clang libclang-dev bison pkg-config gperf curl unzip zip git
        sudo apt-get install -y libc++-dev libva-dev libvdpau-dev libvdpau1 mesa-va-drivers ocl-icd-opencl-dev opencl-headers
        sudo apt-get install -y libpulse-dev libasound-dev libxkbcommon-dev
        sudo apt-get install -y libfontconfig1 libfreetype6
        sudo apt-get install -y libstdc++-12-dev || true
        sudo apt-get install -y libstdc++-8-dev || true
    fi

    mkdir -p {{ExtDir}}

    if [ ! -f "{{OpenCVPath}}/lib/libopencv_core4.a" ]; then
        echo "Installing OpenCV from vcpkg"
        # OpenCV dependencies
        if [ "{{AptGetExists}}" == "true" ]; then
            sudo apt-get install -y libx11-dev libxft-dev libxext-dev autoconf libtool libglfw3 libgles2-mesa-dev libxrandr-dev libxi-dev libxcursor-dev libxdamage-dev libxinerama-dev libxxf86vm-dev libdbus-1-dev libxtst-dev libdbus-1-dev libxi-dev libxtst-dev libsystemd-dev cmake ninja-build
        fi

        # Install vcpkg
        if [ "$IN_DOCKER" != "1" ]; then
            export VCPKG_FORCE_SYSTEM_BINARIES=1
        fi
        git clone --depth 1 https://github.com/Microsoft/vcpkg.git {{ExtDir}}/vcpkg || true

        {{ExtDir}}/vcpkg/bootstrap-vcpkg.sh -disableMetrics

        # Install OpenCV
        {{ExtDir}}/vcpkg/vcpkg install "flatbuffers:{{VcpkgArch}}" --cmake-args=-DVCPKG_BUILD_TYPE=release
        {{ExtDir}}/vcpkg/vcpkg install "opencv4[core,calib3d,opencl,thread,intrinsics]:{{VcpkgArch}}" --cmake-args=-DVCPKG_BUILD_TYPE=release
        rm -rf {{ExtDir}}/vcpkg/buildtrees
        rm -rf {{ExtDir}}/vcpkg/downloads
    fi

    if [[ ! -d "${LIBCLANG_PATH}" ]]; then
        if [ "{{AptGetExists}}" == "true" ]; then
            sudo apt-get install -y libclang-13-dev
        fi
    fi

    pushd {{ExtDir}}

    if [ ! -d "{{ExtDir}}/{{QtVersion}}" ]; then
        echo "Setting up python venv in {{ExtDir}}/venv"
        python3 -m venv "{{ExtDir}}/venv"
        source "{{ExtDir}}/venv/bin/activate"

        echo "Installing Qt {{QtVersion}}"
        # Install Qt
        pip3 install -U pip
        pip3 install -U aqtinstall
        python3 -m aqt install-qt {{QtLinuxArch}} desktop {{QtVersion}}

        # For VMware: sudo apt install libpocl2
    fi

    if [ "$IN_DOCKER" == "1" ]; then
        echo "Installing AppImage builder"

        sudo apt-get install -y debian-keyring debian-archive-keyring
        sudo apt-key adv --refresh-keys --keyserver keyserver.ubuntu.com
        sudo apt-get install -y python3-setuptools patchelf desktop-file-utils libgdk-pixbuf2.0-dev fakeroot strace fuse gtk-update-icon-cache
        export HOST_ARCH=`lscpu | grep Architecture | awk {'print $2'}`
        sudo curl -L https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-${HOST_ARCH}.AppImage -o /usr/local/bin/appimagetool

        sudo chmod +x /usr/local/bin/appimagetool

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

    if [ "{{AptGetExists}}" == "false" ]; then
        echo "Warning: apt-get not found, so build dependencies could not be automatically installed."
        echo "  Make sure you have installed all required build dependencies using your system's package manager."
    fi

# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ Development ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

[no-cd]
run *param:
    cargo run --release -- {{param}}

[no-cd]
test *param:
    cargo test -- {{param}}

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
        sudo docker run -v $PROJECT_DIR:$PROJECT_DIR -v $HOME/.cargo:/root/.cargo debian/eol:buster bash -c "
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
            cargo install --force just
            if [ "$FORCE_ARCH" == "aarch64" ]; then
                export FORCE_ARCH=aarch64
                rustup target add aarch64-unknown-linux-gnu
                apt install -y gcc-aarch64-linux-gnu g++-aarch64-linux-gnu
            fi
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

    rm -rf "$PROJECT_DIR/_deployment/_binaries/{{PackageSuffix}}"

    TARGET="$PROJECT_DIR/_deployment/_binaries/{{PackageSuffix}}"
    mkdir -p $TARGET
    mkdir -p $TARGET/camera_presets
    mkdir -p $TARGET/lib
    mkdir -p $TARGET/plugins
    mkdir -p $TARGET/qml

    cp -f "$QT_DIR/lib/libQt6Core.so.6"                            "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6Gui.so.6"                             "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6Quick.so.6"                           "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6Qml.so.6"                             "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6QmlCore.so.6"                         "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6LabsFolderListModel.so.6"             "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6LabsQmlModels.so.6"                   "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6QuickControls2.so.6"                  "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6QuickControls2Impl.so.6"              "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6QuickControls2Material.so.6"          "$TARGET/lib/" || true
    cp -f "$QT_DIR/lib/libQt6QuickControls2MaterialStyleImpl.so.6" "$TARGET/lib/" || true
    cp -f "$QT_DIR/lib/libQt6QuickControls2Basic.so.6"             "$TARGET/lib/" || true
    cp -f "$QT_DIR/lib/libQt6QuickControls2BasicStyleImpl.so.6"    "$TARGET/lib/" || true
    cp -f "$QT_DIR/lib/libQt6QuickTemplates2.so.6"                 "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6QuickDialogs2.so.6"                   "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6QuickDialogs2QuickImpl.so.6"          "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6QuickDialogs2Utils.so.6"              "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6QuickEffects.so.6"                    "$TARGET/lib/" || true
    cp -f "$QT_DIR/lib/libQt6QuickLayouts.so.6"                    "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6Svg.so.6"                             "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6DBus.so.6"                            "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6QmlModels.so.6"                       "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6QmlWorkerScript.so.6"                 "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6Network.so.6"                         "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6OpenGL.so.6"                          "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6Widgets.so.6"                         "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6XcbQpa.so.6"                          "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6WaylandClient.so.6"                   "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6WaylandEglClientHwIntegration.so.6"   "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6EglFSDeviceIntegration.so.6"          "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6EglFsKmsSupport.so.6"                 "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libQt6WlShellIntegration.so.6"              "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libicudata.so.56"                           "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libicuuc.so.56"                             "$TARGET/lib/"
    cp -f "$QT_DIR/lib/libicui18n.so.56"                           "$TARGET/lib/"

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

    cp -f "$FFMPEG_DIR/lib/{{FFmpegLinuxArch}}/libavcodec.so.61"         "$TARGET/lib/"
    cp -f "$FFMPEG_DIR/lib/{{FFmpegLinuxArch}}/libavfilter.so.10"        "$TARGET/lib/"
    cp -f "$FFMPEG_DIR/lib/{{FFmpegLinuxArch}}/libavformat.so.61"        "$TARGET/lib/"
    cp -f "$FFMPEG_DIR/lib/{{FFmpegLinuxArch}}/libavutil.so.59"          "$TARGET/lib/"
    cp -f "$FFMPEG_DIR/lib/{{FFmpegLinuxArch}}/libswresample.so.5"       "$TARGET/lib/"
    cp -f "$FFMPEG_DIR/lib/{{FFmpegLinuxArch}}/libswscale.so.8"          "$TARGET/lib/"

    cp -f "$CARGO_TARGET/gyroflow"                         "$TARGET/"
    strip "$TARGET/gyroflow"

    # cp -rf "$PROJECT_DIR/resources/camera_presets"         "$TARGET/"
    # rm -rf "$TARGET/camera_presets/.git"
    mkdir -p "$TARGET/camera_presets/"
    cp -f "$PROJECT_DIR/resources/camera_presets/profiles.cbor.gz" "$TARGET/camera_presets/profiles.cbor.gz"

    pushd $TARGET/..
    tar -czf Gyroflow-{{PackageSuffix}}.tar.gz --transform 's!{{PackageSuffix}}!Gyroflow!' {{PackageSuffix}}

    # ---- Build AppImage ----
    export APP_DIR=$TARGET/../AppDir
    export APP_VERSION={{ProjectVersion}}

    rm -rf $APP_DIR
    mkdir -p $APP_DIR/usr/share/icons
    cp -f $PROJECT_DIR/_deployment/linux/gyroflow.png $APP_DIR/usr/share/icons/
    cp -f $PROJECT_DIR/_deployment/linux/gyroflow.svg $APP_DIR/usr/share/icons/

    if [ "$IN_DOCKER" == "1" ]; then
        # Docker doesn't support FUSE
        export APPIMAGE_EXTRACT_AND_RUN=1
    fi

    cp -rf $TARGET/* $APP_DIR/
    if [ "{{Arch}}" == "aarch64" ]; then
        cp $PROJECT_DIR/_deployment/linux/AppImageBuilder.yml AppImageBuilder-arm64.yml
        sed -i 's|x86_64|aarch64|' AppImageBuilder-arm64.yml
        sed -i 's|amd64|aarch64|' AppImageBuilder-arm64.yml
        appimage-builder --recipe AppImageBuilder-arm64.yml
        rm AppImageBuilder-arm64.yml
    else
        appimage-builder --recipe $PROJECT_DIR/_deployment/linux/AppImageBuilder.yml
    fi
    chmod +x Gyroflow-${APP_VERSION}-{{ArchName}}.AppImage

    # Try to fix AppImage permissions
    ./Gyroflow-${APP_VERSION}-{{ArchName}}.AppImage --appimage-extract
    rm -f Gyroflow-${APP_VERSION}-{{ArchName}}.AppImage
    sudo chmod 755 squashfs-root
    find ./squashfs-root -type d -exec sudo chmod 755 {} \;
    mv squashfs-root Gyroflow.AppDir
    ARCH={{Arch}} appimagetool ./Gyroflow.AppDir
    rm -rf Gyroflow.AppDir

    mv Gyroflow-{{ArchName}}.AppImage Gyroflow-{{PackageSuffix}}.AppImage
    # ---- Build AppImage ----

    rm -rf $APP_DIR
    rm -rf $TARGET

    popd
