#!/bin/bash

sudo apt install p7zip-full python3-pip clang libclang-dev bison pkg-config gperf curl git libc++-dev libva-dev libvdpau-dev libvdpau1 mesa-va-drivers intel-opencl-icd ocl-icd-opencl-dev opencl-headers

# OpenCV dependencies
sudo apt install libx11-dev libxft-dev libxext-dev autoconf libtool libglfw3 libgles2-mesa-dev libxrandr-dev libxi-dev libxcursor-dev libxdamage-dev libxinerama-dev

if [ "$1" != "CI" ]; then
    # Install vcpkg
    git clone --depth 1 https://github.com/Microsoft/vcpkg.git
    ./vcpkg/bootstrap-vcpkg.sh

    # Install Qt
    pip3 install -U pip
    pip3 install aqtinstall
    python3 -m aqt install-qt linux desktop 6.2.2

    # Install OpenCV
    ./vcpkg/vcpkg install "opencv[core]:x64-linux-release"

    # For VMware: sudo apt install libpocl2
fi

# Download and extract ffmpeg
curl -L https://sourceforge.net/projects/avbuild/files/linux/ffmpeg-4.4-linux-clang-default.tar.xz/download -o ffmpeg.tar.xz
7z x ffmpeg.tar.xz
tar -xf ffmpeg.tar.xz
cd ..
