#!/bin/bash

if [ "$1" != "CI" ]; then
    brew install p7zip

    # Install vcpkg
    git clone --depth 1 https://github.com/Microsoft/vcpkg.git
    ./vcpkg/bootstrap-vcpkg.sh

    # Install Qt
    pip3 install -U pip
    pip3 install aqtinstall
    python3 -m aqt install-qt mac desktop 6.2.2

    # Install OpenCV
    ./vcpkg/vcpkg install "opencv[core]:x64-osx-release"
    ./vcpkg/vcpkg install "opencv[core]:arm64-osx"
fi

# Download and extract ffmpeg 
curl -L https://sourceforge.net/projects/avbuild/files/macOS/ffmpeg-4.4-macOS-lite.tar.xz/download -o ffmpeg.tar.xz
7z x ffmpeg.tar.xz
7z x ffmpeg.tar
mkdir -p ffmpeg-x86_64/lib
mkdir -p ffmpeg-arm64/lib
cd ffmpeg-4.4-macOS-lite
lipo lib/libavcodec.a    -thin x86_64 -output ../ffmpeg-x86_64/lib/libavcodec.a
lipo lib/libavformat.a   -thin x86_64 -output ../ffmpeg-x86_64/lib/libavformat.a
lipo lib/libavdevice.a   -thin x86_64 -output ../ffmpeg-x86_64/lib/libavdevice.a
lipo lib/libavfilter.a   -thin x86_64 -output ../ffmpeg-x86_64/lib/libavfilter.a
lipo lib/libavutil.a     -thin x86_64 -output ../ffmpeg-x86_64/lib/libavutil.a
lipo lib/libswresample.a -thin x86_64 -output ../ffmpeg-x86_64/lib/libswresample.a
lipo lib/libswscale.a    -thin x86_64 -output ../ffmpeg-x86_64/lib/libswscale.a

lipo lib/libavcodec.a    -thin arm64 -output ../ffmpeg-arm64/lib/libavcodec.a
lipo lib/libavformat.a   -thin arm64 -output ../ffmpeg-arm64/lib/libavformat.a
lipo lib/libavdevice.a   -thin arm64 -output ../ffmpeg-arm64/lib/libavdevice.a
lipo lib/libavfilter.a   -thin arm64 -output ../ffmpeg-arm64/lib/libavfilter.a
lipo lib/libavutil.a     -thin arm64 -output ../ffmpeg-arm64/lib/libavutil.a
lipo lib/libswresample.a -thin arm64 -output ../ffmpeg-arm64/lib/libswresample.a
lipo lib/libswscale.a    -thin arm64 -output ../ffmpeg-arm64/lib/libswscale.a
cp -R include ../ffmpeg-x86_64/include
cp -R include ../ffmpeg-arm64/include
cd ..
