#!/bin/bash

if [ "$1" != "CI" ]; then
    brew install p7zip pkg-config
    rustup target add aarch64-apple-ios

    # Install vcpkg
    git clone --depth 1 https://github.com/Microsoft/vcpkg.git
    ./vcpkg/bootstrap-vcpkg.sh -disableMetrics

    # Install Qt
    pip3 install -U pip
    pip3 install aqtinstall
    python3 -m aqt install-qt mac ios 6.4.0

    # Install OpenCV
    ./vcpkg/vcpkg install "opencv[core]:arm64-ios"
fi

if [ ! -d "ffmpeg-arm64-ios" ]; then
    # Download and extract ffmpeg
    curl -L https://sourceforge.net/projects/avbuild/files/iOS/ffmpeg-5.1-iOS-gpl-lite.tar.xz/download -o ffmpeg.tar.xz
    7z x ffmpeg.tar.xz
    tar -xf ffmpeg.tar
    mv -f ffmpeg-5.1-iOS-gpl-lite ffmpeg-arm64-ios
    cd ..
fi