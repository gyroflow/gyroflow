# Qt
$Env:Path += ";$PSScriptRoot\ext\6.2.3\msvc2019_64\bin"

# FFmpeg
$Env:FFMPEG_DIR = "$PSScriptRoot\ext\ffmpeg-4.4-windows-desktop-clang-gpl-lite"
$Env:Path += ";$FFMPEG_DIR\bin"

# OpenCV
$Env:OPENCV_LINK_LIBS = "opencv_core,opencv_calib3d,opencv_features2d,opencv_imgproc,opencv_video,opencv_flann,opencv_imgcodecs,opencv_objdetect"
$Env:OPENCV_LINK_LIBS += ",opencv_dnn,opencv_ml,opencv_highgui" # needed for debug build
$Env:OPENCV_LINK_PATHS = "$PSScriptRoot\ext\vcpkg\installed\x64-windows-release\lib"
$Env:OPENCV_INCLUDE_PATHS = "$PSScriptRoot\ext\vcpkg\installed\x64-windows-release\include"
$Env:Path += ";$PSScriptRoot\ext\vcpkg\installed\x64-windows-release\bin"

# Clang
$Env:LIBCLANG_PATH = "$PSScriptRoot\ext\llvm-13-win64\bin"
$Env:Path += ";$PSScriptRoot\ext\llvm-13-win64\bin"
