# Qt
$Env:Path = "$PSScriptRoot\ext\6.3.0\msvc2019_64\bin;$Env:Path"

# FFmpeg
$Env:FFMPEG_DIR = "$PSScriptRoot\ext\ffmpeg-5.0-windows-desktop-vs2022-gpl-lite"
$Env:Path += ";$Env:FFMPEG_DIR\bin"

# OpenCV
$Env:OPENCV_LINK_LIBS = "opencv_core,opencv_calib3d,opencv_features2d,opencv_imgproc,opencv_video,opencv_flann,opencv_imgcodecs,opencv_objdetect"
$Env:OPENCV_LINK_LIBS += ",opencv_dnn,opencv_ml,opencv_highgui" # needed for debug build
$Env:OPENCV_LINK_PATHS = "$PSScriptRoot\ext\vcpkg\installed\x64-windows-release\lib"
$Env:OPENCV_INCLUDE_PATHS = "$PSScriptRoot\ext\vcpkg\installed\x64-windows-release\include"
$Env:Path = ";$PSScriptRoot\ext\vcpkg\installed\x64-windows-release\bin;$Env:Path"

# Clang
$Env:LIBCLANG_PATH = "$PSScriptRoot\ext\llvm-13-win64\bin"
$Env:Path = "$PSScriptRoot\ext\llvm-13-win64\bin;$Env:Path"
# $Env:LIBCLANG_PATH = "D:\Program Files\LLVM\bin"
# $Env:Path = "D:\Program Files\LLVM\bin;$Env:Path" # or other path if you have LLVM installed in other place

# 7z
# $Env:7Z_PATH = "C:\Program Files\7-Zip" # or other path if you have 7zip installed in other place
# $Env:Path = "C:\Program Files\7-Zip;$Env:Path"
