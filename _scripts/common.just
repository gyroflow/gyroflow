set windows-shell := ["powershell.exe", "-NoLogo", "-Command"]
PathSep := if os() == "windows" { ";" } else { ":" }

ProjectVersion := "1.6.1"
ProjectDir := if path_exists(justfile_directory() / "Cargo.toml") == "true" { justfile_directory()
       } else if path_exists(justfile_directory() / ".." / "Cargo.toml") == "true" { justfile_directory() / "../"
       } else { justfile_directory() }
ExtDir := ProjectDir / "ext"
Arch := env("FORCE_ARCH", arch())

# ――――――――――――――――――――――――――――――――――――――――― Qt ――――――――――――――――――――――――――――――――――――――――――
QtVersion := env_var_or_default("FORCE_QT_VERSION", if os() == "windows" { "6.7.3" } else { if Arch == "aarch64" { "6.7.3" } else { "6.4.3" } })
QtArch := if os() + Arch == "windowsx86_64"  { "msvc2019_64"
   } else if os() + Arch == "windowsaarch64" { "msvc2019_arm64"
   } else if os() == "macos"   { "macos"
   } else if Arch == "aarch64" { "gcc_arm64"
   } else                      { "gcc_64" }
QtPath := ProjectDir / "ext/" / QtVersion / QtArch / "bin"
export QMAKE := env_var_or_default("QMAKE", if path_exists(QtPath / "qmake.bat") == "true" { QtPath / "qmake.bat" } else { QtPath / "qmake" })
# ――――――――――――――――――――――――――――――――――――――――― Qt ――――――――――――――――――――――――――――――――――――――――――

# ――――――――――――――――――――――――――――――――――――――― FFmpeg ――――――――――――――――――――――――――――――――――――――――
FFmpeg := if os() == "windows"               { "ffmpeg-master-windows-desktop-clang-nvenc12abi-gpl-lite"
   } else if os() + Arch == "macosaarch64"   { "ffmpeg-arm64"
   } else if os() + Arch == "macosx86_64"    { "ffmpeg-x86_64"
   } else                                    { "ffmpeg-master-linux-clang-nvenc12abi-gpl-lite" }
export FFMPEG_DIR := ProjectDir / "ext" / FFmpeg
export FFMPEG_ARCH := if Arch == "aarch64" { "arm64" } else { if os() == "windows" { "x64" } else { "amd64" } }
# ――――――――――――――――――――――――――――――――――――――― FFmpeg ――――――――――――――――――――――――――――――――――――――――

# ――――――――――――――――――――――――――――――――――――――― OpenCV ――――――――――――――――――――――――――――――――――――――――
VcpkgArch := if os() + Arch == "windowsx86_64"  { "x64-windows"
      } else if os() + Arch == "windowsaarch64" { "arm64-windows"
      } else if os() + Arch == "macosaarch64"   { "arm64-osx"
      } else if os() + Arch == "macosx86_64"    { "x64-osx"
      } else if Arch == "aarch64"               { "arm64-linux"
      } else                                    { "x64-linux" }
export OPENCV_LINK_LIBS := "opencv_core4,opencv_calib3d4,opencv_features2d4,opencv_imgproc4,opencv_video4,opencv_flann4,opencv_imgcodecs4,opencv_objdetect4,opencv_stitching4" + if os() == "linux" { if Arch == "aarch64" { ",tegra_hal,png" } else { ",png" } } else { "" }
export OPENCV_LINK_PATHS := ProjectDir / "ext/vcpkg/installed" / VcpkgArch / "lib," + ProjectDir / "ext/vcpkg/installed" / VcpkgArch / "lib/manual-link/opencv4_thirdparty"
export OPENCV_INCLUDE_PATHS := ProjectDir / "ext/vcpkg/installed" / VcpkgArch / "include/opencv4"
OpenCVPath := ProjectDir / "ext/vcpkg/installed" / VcpkgArch
# ――――――――――――――――――――――――――――――――――――――― OpenCV ――――――――――――――――――――――――――――――――――――――――

# ――――――――――――――――――――――――――――――――――――――― MacOS ―――――――――――――――――――――――――――――――――――――――――
export DYLD_FALLBACK_LIBRARY_PATH := if os() == "macos" { if path_exists(`xcode-select --print-path` + "/Toolchains/XcodeDefault.xctoolchain/usr/lib/") == "true" { `xcode-select --print-path` + "/Toolchains/XcodeDefault.xctoolchain/usr/lib/" } else { `xcode-select --print-path` + "/usr/lib/" } } else { "" }
export MACOSX_DEPLOYMENT_TARGET := "10.15"
# ――――――――――――――――――――――――――――――――――――――― MacOS ―――――――――――――――――――――――――――――――――――――――――

# ――――――――――――――――――――――――――――――――――――――― Clang ―――――――――――――――――――――――――――――――――――――――――
LlvmConfigExists := if os() == "linux" { `which llvm-config > /dev/null && echo true || echo false` } else { "false" }
export LIBCLANG_PATH := if os() == "macos" {
   DYLD_FALLBACK_LIBRARY_PATH
} else {
   if path_exists(ExtDir / "llvm/bin") == "true" {
      ExtDir / "llvm/bin"
   } else {
      env_var_or_default("LIBCLANG_PATH", if "{{LlvmConfigExists}}" == "true" {
         `llvm-config --libdir`
      } else {
         if path_exists("/usr/lib/llvm-13/lib/") == "true" { "/usr/lib/llvm-13/lib/" } else { "" }
      })
   }
}
LLVMPath := LIBCLANG_PATH
# ――――――――――――――――――――――――――――――――――――――― Clang ―――――――――――――――――――――――――――――――――――――――――


export LD_LIBRARY_PATH := ProjectDir / "target/release:" + QtPath / "../lib:" + FFMPEG_DIR + "/lib:" + FFMPEG_DIR + "/lib/" + FFMPEG_ARCH

export PATH := QtPath + PathSep + FFMPEG_DIR + "/bin" + PathSep + FFMPEG_DIR + "/bin/" + FFMPEG_ARCH + PathSep + OpenCVPath + "/bin" + PathSep + LLVMPath + PathSep + env_var('PATH')

# export VULKAN_SDK := "D:/Programy/VulkanSDK/1.3.236.0"
