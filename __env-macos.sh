#!/bin/bash

export PROJECT_DIR=$(cd "$(dirname "$0")"; pwd -P)
export OPENCV_LINK_LIBS="opencv_core,opencv_calib3d,opencv_features2d,opencv_imgproc,opencv_video,opencv_flann"

ARCH=x64_64
ARCH_VCPKG=x64-osx-release
if [ $(uname -m) = "arm64" ]; then
    ARCH=arm64
    ARCH_VCPKG=arm64-osx
    export OPENCV_LINK_LIBS="$OPENCV_LINK_LIBS,tegra_hal"
fi

export FFMPEG_DIR=$PROJECT_DIR/ext/ffmpeg-$ARCH
export OPENCV_LINK_PATHS=$PROJECT_DIR/ext/vcpkg/installed/$ARCH_VCPKG/lib
export OPENCV_INCLUDE_PATHS=$PROJECT_DIR/ext/vcpkg/installed/$ARCH_VCPKG/include/

export PATH="$PROJECT_DIR/ext/6.3.1/macos/bin:$PATH"

#export DYLD_FALLBACK_LIBRARY_PATH="$(xcode-select --print-path)/usr/lib/"
export DYLD_FALLBACK_LIBRARY_PATH="$(xcode-select --print-path)/Toolchains/XcodeDefault.xctoolchain/usr/lib/"

export MACOSX_DEPLOYMENT_TARGET="10.11"
