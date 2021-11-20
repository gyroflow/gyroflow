# Install nightly: rustup default nightly
# cargo install --git https://github.com/zer0def/android-ndk-rs.git cargo-apk
# rustup target add aarch64-linux-android
# update Cargo.toml to remove default opencl feature and remove [[bin]] and uncomment [lib]

# Add to cc/src/lib.rs cmd.arg("-std=c++17"); before run(); (in compile_object)
# Add alignment 16 to cpp_macros/src/lib.rs
# Add  .clang_arg("-I$LIBCLANG_PATH/../lib/clang/13.0.0/include")
#      .clang_arg("--sysroot=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/windows-x86_64/sysroot")
#      .clang_arg("--target=aarch64-linux-android")
# to ffmpeg-sys-next-4.4.0-next.2\build.rs

$Env:Path += ";D:\Programy\Qt\6.2.1\android_arm64_v8a\bin"
$Env:Path += ";D:\Programy\Qt\6.2.1\mingw81_64\bin\"
$Env:ANDROID_NDK_HOME = "D:\Programy\Android\sdk\ndk-bundle"
$Env:ANDROID_SDK_ROOT = "D:\Programy\Android\sdk\"
$Env:JAVA_HOME = "D:\Programy\Java\jdk1.8.0_141"
$Env:QMAKE = "D:\Programy\Qt\6.2.1\android_arm64_v8a\bin\qmake.bat"
$Env:FFMPEG_DIR = "$PSScriptRoot\ext\ffmpeg-4.4-android-lite"
$Env:LIBCLANG_PATH = "$PSScriptRoot\ext\llvm-13-win64\bin"
$Env:OPENCV_LINK_LIBS = "opencv_calib3d,opencv_features2d,opencv_imgproc,opencv_video,opencv_flann,opencv_core,tegra_hal,tbb,ittnotify,z"
$Env:OPENCV_LINK_PATHS = "$PSScriptRoot\ext\OpenCV-android-sdk\sdk\native\staticlibs\arm64-v8a,$PSScriptRoot\ext\OpenCV-android-sdk\sdk\native\3rdparty\libs\arm64-v8a"
$Env:OPENCV_INCLUDE_PATHS = "$PSScriptRoot\ext\OpenCV-android-sdk\sdk\native\jni\include"

cargo apk build --release

mkdir "$PSScriptRoot\target\android-build" -ErrorAction SilentlyContinue
mkdir "$PSScriptRoot\target\android-build\libs" -ErrorAction SilentlyContinue
Copy-Item -Path "$PSScriptRoot\target\release\apk\lib\*" -Destination "$PSScriptRoot\target\android-build\libs\" -Recurse -Force
Copy-Item -Path "$PSScriptRoot\android\src" -Destination "$PSScriptRoot\target\android-build\" -Recurse -Force
Copy-Item -Path "$PSScriptRoot\\target\aarch64-linux-android\release\libffmpeg.so" -Destination "$PSScriptRoot\target\android-build\libs\" -Force
Move-Item -Path "$PSScriptRoot\target\android-build\libs\arm64-v8a\libgyroflow.so" -Destination "$PSScriptRoot\target\android-build\libs\arm64-v8a\libgyroflow_arm64-v8a.so" -Force

androiddeployqt --input "$PSScriptRoot\android\android-deploy.json" `
                --output "$PSScriptRoot\target\android-build" `
                --deployment bundled `
                --android-platform android-30 `
                --jdk ${Env:JAVA_HOME} `
                --gradle

adb install "$PSScriptRoot\target\android-build\build\outputs\apk\debug\android-build-debug.apk"

#--Alternative
#--cargo install cargo-ndk
#--cargo ndk -t arm64-v8a --platform 26 -o ./jniLibs build --release