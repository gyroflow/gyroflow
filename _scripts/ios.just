!include common.just

QtVersionIOS := "6.5.0"

# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ Dependencies ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

[no-cd]
install-deps:
    #!/bin/bash
    brew install p7zip pkg-config
    rustup target add aarch64-apple-ios
    xcode-select --install
    export DYLD_FALLBACK_LIBRARY_PATH=""
    export MACOSX_DEPLOYMENT_TARGET=""

    cd {{ExtDir}}

    # Install vcpkg
    git clone --depth 1 https://github.com/Microsoft/vcpkg.git
    ./vcpkg/bootstrap-vcpkg.sh -disableMetrics

    if [ ! -d "{{ExtDir}}/{{QtVersionIOS}}/ios" ]; then
        # Install Qt
        pip3 install -U pip aqtinstall
        python3 -m aqt install-qt mac ios {{QtVersionIOS}}
    fi

    if [ ! -f "{{ExtDir}}/vcpkg/installed/arm64-ios/lib/libopencv_core4.a" ]; then
        echo "Installing OpenCV from vcpkg"
        # Install OpenCV
        ./vcpkg/vcpkg install "opencv4[core]:arm64-ios"
    fi

    if [ ! -d "ffmpeg-master-iOS-gpl-lite" ]; then
        echo "Downloading ffmpeg for iOS"

        # Download and extract ffmpeg
        curl -L https://sourceforge.net/projects/avbuild/files/iOS/ffmpeg-master-iOS-gpl-lite.tar.xz/download -o ffmpeg.tar.xz
        7z x -aoa ffmpeg.tar.xz
        tar -xf ffmpeg.tar
        rm ffmpeg.tar ffmpeg.tar.xz
    fi

# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ Development ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

#[no-cd]
#run *param:
#    cargo run --release -- {{param}}
#
#[no-cd]
#debug *param:
#    cargo run -- {{param}}

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

