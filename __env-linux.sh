#!/bin/bash

export PROJECT_DIR=$(dirname $(readlink -f $0))

export FFMPEG_DIR=$PROJECT_DIR/ext/ffmpeg-5.0-linux-clang-gpl-lite
export OPENCV_LINK_PATHS=$PROJECT_DIR/ext/vcpkg/installed/x64-linux-release/lib
export OPENCV_INCLUDE_PATHS=$PROJECT_DIR/ext/vcpkg/installed/x64-linux-release/include/

export PATH="$PROJECT_DIR/ext/6.2.3/gcc_64/bin:$FFMPEG_DIR/bin/amd64:$PATH"
export OPENCV_LINK_LIBS="opencv_core,opencv_calib3d,opencv_features2d,opencv_imgproc,opencv_video,opencv_flann"

export LD_LIBRARY_PATH="$PROJECT_DIR/target/release:$PROJECT_DIR/ext/6.2.3/gcc_64/lib:$FFMPEG_DIR/lib:$FFMPEG_DIR/lib/amd64"

# Launch same shell with environment set
exec $SHELL -i