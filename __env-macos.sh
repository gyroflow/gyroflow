#!/bin/bash
# xcode-select --install
# brew install qt opencv p7zip

PROJECT_DIR="/Users/Admin/gyroflow"

export PATH="$PROJECT_DIR/ext/6.2.2/macos/bin:$PATH"
export FFMPEG_DIR=$PROJECT_DIR/ext/ffmpeg-4.4-macOS-default

export OPENCV_LINK_LIBS="opencv_core,opencv_calib3d,opencv_features2d,opencv_imgproc,opencv_video,opencv_flann"
export OPENCV_LINK_PATHS=$PROJECT_DIR/ext/vcpkg/installed/x64-osx-release/lib
export OPENCV_INCLUDE_PATHS=$PROJECT_DIR/ext/vcpkg/installed/x64-osx-release/include/
#export DYLD_FALLBACK_LIBRARY_PATH="$(xcode-select --print-path)/usr/lib/"
export DYLD_FALLBACK_LIBRARY_PATH="$(xcode-select --print-path)/Toolchains/XcodeDefault.xctoolchain/usr/lib/"
#export LD_LIBRARY_PATH="$PROJECT_DIR/ext/6.2.2/macos/lib"
export MACOSX_DEPLOYMENT_TARGET="10.11"

# Rust cannot link to fat libraries, so extract the architectures to a single files
# This needs to be done once for the downloaded ffmpeg libraries
# lipo libavcodec.a    -thin x86_64 -output libavcodec-x86_64.a     ;  mv libavcodec-x86_64.a    libavcodec.a
# lipo libavformat.a   -thin x86_64 -output libavformat-x86_64.a    ;  mv libavformat-x86_64.a   libavformat.a
# lipo libavdevice.a   -thin x86_64 -output libav-device-x86_64.a   ;  mv libav-device-x86_64.a  libavdevice.a
# lipo libavfilter.a   -thin x86_64 -output libavfilter-x86_64.a    ;  mv libavfilter-x86_64.a   libavfilter.a
# lipo libavutil.a     -thin x86_64 -output libavutil-x86_64.a      ;  mv libavutil-x86_64.a     libavutil.a
# lipo libswresample.a -thin x86_64 -output libswresample-x86_64.a  ;  mv libswresample-x86_64.a libswresample.a
# lipo libswscale.a    -thin x86_64 -output libswscale-x86_64.a     ;  mv libswscale-x86_64.a    libswscale.a
