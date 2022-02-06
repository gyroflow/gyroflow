$Env:Path += ";$PSScriptRoot\ext\6.2.3\msvc2019_64\bin"


$Env:Path += ";$PSScriptRoot\ext\ffmpeg-4.4-windows-desktop-clang-gpl-lite\bin"
$Env:FFMPEG_DIR = "$PSScriptRoot\ext\ffmpeg-4.4-windows-desktop-clang-gpl-lite"

$Env:OPENCV_LINK_LIBS = "opencv_core,opencv_calib3d,opencv_features2d,opencv_imgproc,opencv_video,opencv_flann,opencv_imgcodecs,opencv_objdetect"
$Env:OPENCV_LINK_PATHS = "$PSScriptRoot\ext\vcpkg\installed\x64-windows-release\lib"
$Env:OPENCV_INCLUDE_PATHS = "$PSScriptRoot\ext\vcpkg\installed\x64-windows-release\include"
$Env:Path += ";$PSScriptRoot\ext\vcpkg\installed\x64-windows-release\bin"

$Env:LIBCLANG_PATH = "$PSScriptRoot\ext\llvm-13-win64\bin"
$Env:Path += ";$PSScriptRoot\ext\llvm-13-win64\bin"


# For OpenCV installed by official installer
#$Env:OPENCV_LINK_LIBS = "opencv_world453"
#$Env:OPENCV_LINK_PATHS = "$PSScriptRoot\ext\opencv\453\lib"
#$Env:OPENCV_INCLUDE_PATHS = "$PSScriptRoot\ext\opencv\453\include"
#$Env:Path += ";$PSScriptRoot\ext\opencv\453\bin"
