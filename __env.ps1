# Qt
$Env:Path = "$PSScriptRoot\ext\6.4.0\msvc2019_64\bin;$Env:Path"

# FFmpeg
$Env:FFMPEG_DIR = "$PSScriptRoot\ext\ffmpeg-5.1-windows-desktop-vs2022-gpl-lite"
$Env:Path += ";$Env:FFMPEG_DIR\bin"

# OpenCV
$Env:OPENCV_LINK_LIBS = "opencv_core4,opencv_calib3d4,opencv_features2d4,opencv_imgproc4,opencv_video4,opencv_flann4,opencv_imgcodecs4,opencv_objdetect4"
$Env:OPENCV_LINK_LIBS += ",opencv_dnn4,opencv_ml4,opencv_highgui4,opencv_videoio4" # needed for debug build
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
