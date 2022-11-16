#!/bin/bash

export PROJECT_DIR=$(dirname $(readlink -f $0))

export FFMPEG_DIR=$PROJECT_DIR/ext/ffmpeg-5.1-linux-clang-gpl-lite
export OPENCV_LINK_PATHS=$PROJECT_DIR/ext/vcpkg/installed/x64-linux-release/lib
export OPENCV_INCLUDE_PATHS=$PROJECT_DIR/ext/vcpkg/installed/x64-linux-release/include/

export PATH="$PROJECT_DIR/ext/6.4.1/gcc_64/bin:$FFMPEG_DIR/bin/amd64:$PATH"
export OPENCV_LINK_LIBS="opencv_calib3d4,opencv_features2d4,opencv_imgproc4,opencv_video4,opencv_flann4,opencv_core4"

export LD_LIBRARY_PATH="$PROJECT_DIR/target/release:$PROJECT_DIR/ext/6.4.1/gcc_64/lib:$FFMPEG_DIR/lib:$FFMPEG_DIR/lib/amd64"

# Launch same shell with environment set
exec $SHELL -i