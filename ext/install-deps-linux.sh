#!/bin/bash

sudo apt-get install -y p7zip-full python3-pip clang libclang-dev bison pkg-config gperf curl unzip zip git
sudo apt-get install -y libc++-dev libva-dev libvdpau-dev libvdpau1 mesa-va-drivers ocl-icd-opencl-dev opencl-headers libpulse-dev libasound-dev libxkbcommon-dev

if [ "$1" != "CI" ] && [ "$1" != "docker" ]; then
    # Install vcpkg
    git clone --depth 1 https://github.com/Microsoft/vcpkg.git
    ./vcpkg/bootstrap-vcpkg.sh
    export VCPKG_ROOT=$PWD/vcpkg

    # Install Qt
    pip3 install -U pip
    pip3 install aqtinstall
    python3 -m aqt install-qt linux desktop 6.2.2

    # For VMware: sudo apt install libpocl2
fi

if [ "$1" != "CI" ] || [ "$1" == "docker" ]; then
    # OpenCV dependencies
    sudo apt-get install -y libx11-dev libxft-dev libxext-dev autoconf libtool libglfw3 libgles2-mesa-dev libxrandr-dev libxi-dev libxcursor-dev libxdamage-dev libxinerama-dev libxxf86vm-dev

    # Install OpenCV
    $VCPKG_ROOT/vcpkg install "opencv[core]:x64-linux-release"
fi

# Download and extract ffmpeg
curl -L https://sourceforge.net/projects/avbuild/files/linux/ffmpeg-4.4-linux-clang-default.tar.xz/download -o ffmpeg.tar.xz
7z x -aoa ffmpeg.tar.xz
tar -xf ffmpeg.tar.xz

cd ..