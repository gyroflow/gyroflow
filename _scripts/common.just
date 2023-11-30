set windows-shell := ["powershell.exe", "-NoLogo", "-Command"]
PathSep := if os() == "windows" { ";" } else { ":" }

ProjectVersion := "1.5.4"
ProjectDir := if path_exists(justfile_directory() / "Cargo.toml") == "true" { justfile_directory()
       } else if path_exists(justfile_directory() / ".." / "Cargo.toml") == "true" { justfile_directory() / "../"
       } else { justfile_directory() }
ExtDir := ProjectDir / "ext"

# ――――――――――――――――――――――――――――――――――――――――― Qt ――――――――――――――――――――――――――――――――――――――――――
QtVersion := if os() == "windows" { "6.6.1" } else { env_var_or_default("FORCE_QT_VERSION", "6.4.3") }
QtArch := if os() == "windows" { "msvc2019_64"
   } else if os() == "macos"   { "macos"
   } else                      { "gcc_64" }
QtPath := ProjectDir / "ext/" / QtVersion / QtArch / "bin"
# ――――――――――――――――――――――――――――――――――――――――― Qt ――――――――――――――――――――――――――――――――――――――――――

# ――――――――――――――――――――――――――――――――――――――― FFmpeg ――――――――――――――――――――――――――――――――――――――――
FFmpeg := if os() == "windows" { "ffmpeg-6.1-windows-desktop-vs2022-gpl-lite"
   } else if os() + arch() == "macosaarch64" { "ffmpeg-arm64"
   } else if os() + arch() == "macosx86_64"  { "ffmpeg-x64_64"
   } else                      { "ffmpeg-6.1-linux-clang-gpl-lite" }
export FFMPEG_DIR := ProjectDir / "ext" / FFmpeg
# ――――――――――――――――――――――――――――――――――――――― FFmpeg ――――――――――――――――――――――――――――――――――――――――

# ――――――――――――――――――――――――――――――――――――――― OpenCV ――――――――――――――――――――――――――――――――――――――――
VcpkgArch := if os() == "windows"               { "x64-windows-release"
      } else if os() + arch() == "macosaarch64" { "arm64-osx"
      } else if os() + arch() == "macosx86_64"  { "x64-osx-release"
      } else                                    { "x64-linux-release" }
export OPENCV_LINK_LIBS := "opencv_core4,opencv_calib3d4,opencv_features2d4,opencv_imgproc4,opencv_video4,opencv_flann4,opencv_imgcodecs4,opencv_objdetect4,opencv_stitching4" + if os() == "linux" { ",png" } else if os() + arch() == "macosaarch64" { ",tegra_hal" } else { "" }
export OPENCV_LINK_PATHS := ProjectDir / "ext/vcpkg/installed" / VcpkgArch / "lib," + ProjectDir / "ext/vcpkg/installed" / VcpkgArch / "lib/manual-link/opencv4_thirdparty"
export OPENCV_INCLUDE_PATHS := ProjectDir / "ext/vcpkg/installed" / VcpkgArch / "include"
OpenCVPath := ProjectDir / "ext/vcpkg/installed" / VcpkgArch / "bin"
# ――――――――――――――――――――――――――――――――――――――― OpenCV ――――――――――――――――――――――――――――――――――――――――

# ――――――――――――――――――――――――――――――――――――――― MacOS ―――――――――――――――――――――――――――――――――――――――――
export DYLD_FALLBACK_LIBRARY_PATH := if os() == "macos" { if path_exists(`xcode-select --print-path` + "/Toolchains/XcodeDefault.xctoolchain/usr/lib/") == "true" { `xcode-select --print-path` + "/Toolchains/XcodeDefault.xctoolchain/usr/lib/" } else { `xcode-select --print-path` + "/usr/lib/" } } else { "" }
export MACOSX_DEPLOYMENT_TARGET := "10.15"
# ――――――――――――――――――――――――――――――――――――――― MacOS ―――――――――――――――――――――――――――――――――――――――――

# ――――――――――――――――――――――――――――――――――――――― Clang ―――――――――――――――――――――――――――――――――――――――――
export LIBCLANG_PATH := if os() == "macos" { DYLD_FALLBACK_LIBRARY_PATH } else { if path_exists(ExtDir / "llvm/bin") == "true" { ExtDir / "llvm/bin" } else { env_var_or_default("LIBCLANG_PATH", if path_exists("/usr/lib/llvm-13/lib/") == "true" { "/usr/lib/llvm-13/lib/" } else { "" }) } }
LLVMPath := LIBCLANG_PATH
# ――――――――――――――――――――――――――――――――――――――― Clang ―――――――――――――――――――――――――――――――――――――――――

export LD_LIBRARY_PATH := ProjectDir / "target/release:" + QtPath / "../lib:" + FFMPEG_DIR + "/lib:" + FFMPEG_DIR + "/lib/amd64"

export PATH := QtPath + PathSep + FFMPEG_DIR + "/bin" + PathSep + FFMPEG_DIR + "/bin/amd64" + PathSep + OpenCVPath + PathSep + LLVMPath + PathSep + env_var('PATH')

# export VULKAN_SDK := "D:/Programy/VulkanSDK/1.3.236.0"
