#!/bin/bash

export PROJECT_DIR=$(cd "$(dirname "$0")"; pwd -P)
export OPENCV_LINK_LIBS="opencv_core4,opencv_calib3d4,opencv_features2d4,opencv_imgproc4,opencv_video4,opencv_flann4"

export FFMPEG_DIR=$PROJECT_DIR/ext/ffmpeg-arm64-ios
export OPENCV_LINK_PATHS=$PROJECT_DIR/ext/vcpkg/installed/arm64-ios/lib
export OPENCV_INCLUDE_PATHS=$PROJECT_DIR/ext/vcpkg/installed/arm64-ios/include/

export PATH="$PROJECT_DIR/ext/6.3.1/ios/bin:$PATH"

IPHONESDK="$(xcode-select -p)/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk"
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_apple_ios="--target=arm64-apple-ios -arch arm64 -miphoneos-version-min=15 -isysroot $IPHONESDK"
export CFLAGS_aarch64_apple_darwin="-mmacosx-version-min=10.14"

export IPHONEOS_DEPLOYMENT_TARGET="15.0"
