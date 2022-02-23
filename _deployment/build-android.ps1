$PROJECT_DIR="$PSScriptRoot\.."

$QT_LIBS = "D:\Programy\Qt\6.2.3\android_arm64_v8a\lib"
$Env:Path += ";D:\Programy\Qt\6.2.3\android_arm64_v8a\bin"
$Env:Path += ";D:\Programy\Qt\6.2.3\mingw_64\bin\"
$Env:ANDROID_NDK_HOME = "D:\Programy\Android\sdk\ndk-bundle"
$Env:ANDROID_SDK_ROOT = "D:\Programy\Android\sdk\"
$Env:JAVA_HOME = "D:\Programy\Java\jdk-14.0.1"
$Env:QMAKE = "D:\Programy\Qt\6.2.3\android_arm64_v8a\bin\qmake.bat"
$Env:FFMPEG_DIR = "$PROJECT_DIR\ext\ffmpeg-5.0-android-gpl-lite"
$Env:LIBCLANG_PATH = "$PROJECT_DIR\ext\llvm-13-win64\bin"
$Env:OPENCV_LINK_LIBS = "opencv_calib3d,opencv_features2d,opencv_imgproc,opencv_video,opencv_flann,opencv_core,tegra_hal,tbb,ittnotify,z"
$Env:OPENCV_LINK_PATHS = "$PROJECT_DIR\ext\OpenCV-android-sdk\sdk\native\staticlibs\arm64-v8a,$PROJECT_DIR\ext\OpenCV-android-sdk\sdk\native\3rdparty\libs\arm64-v8a"
$Env:OPENCV_INCLUDE_PATHS = "$PROJECT_DIR\ext\OpenCV-android-sdk\sdk\native\jni\include"

$CLANG_LIB = $Env:LIBCLANG_PATH.replace('\', '/').replace('/bin', '/lib');
$NDK_REPLACED = $Env:ANDROID_NDK_HOME.replace('\', '/');
$Env:BINDGEN_EXTRA_CLANG_ARGS = "-I$CLANG_LIB/clang/13.0.0/include --sysroot=$NDK_REPLACED/toolchains/llvm/prebuilt/windows-x86_64/sysroot"

Copy-Item -Path "$QT_LIBS\libQt6Core_arm64-v8a.so"    -Destination "$QT_LIBS\libQt6Core.so"    -ErrorAction SilentlyContinue
Copy-Item -Path "$QT_LIBS\libQt6Gui_arm64-v8a.so"     -Destination "$QT_LIBS\libQt6Gui.so"     -ErrorAction SilentlyContinue
Copy-Item -Path "$QT_LIBS\libQt6Widgets_arm64-v8a.so" -Destination "$QT_LIBS\libQt6Widgets.so" -ErrorAction SilentlyContinue
Copy-Item -Path "$QT_LIBS\libQt6Quick_arm64-v8a.so"   -Destination "$QT_LIBS\libQt6Quick.so"   -ErrorAction SilentlyContinue
Copy-Item -Path "$QT_LIBS\libQt6Qml_arm64-v8a.so"     -Destination "$QT_LIBS\libQt6Qml.so"     -ErrorAction SilentlyContinue
Copy-Item -Path "$QT_LIBS\libQt6QuickControls2_arm64-v8a.so" -Destination "$QT_LIBS\libQt6QuickControls2.so" -ErrorAction SilentlyContinue

cargo apk build --release

mkdir "$PROJECT_DIR\target\android-build" -ErrorAction SilentlyContinue
mkdir "$PROJECT_DIR\target\android-build\libs" -ErrorAction SilentlyContinue
Copy-Item -Path "$PROJECT_DIR\target\release\apk\lib\*" -Destination "$PROJECT_DIR\target\android-build\libs\" -Recurse -Force
Copy-Item -Path "$PROJECT_DIR\_deployment\android\src" -Destination "$PROJECT_DIR\target\android-build\" -Recurse -Force
Copy-Item -Path "$PROJECT_DIR\target\aarch64-linux-android\release\libffmpeg.so" -Destination "$PROJECT_DIR\target\android-build\libs\arm64-v8a\" -Force
Move-Item -Path "$PROJECT_DIR\target\android-build\libs\arm64-v8a\libgyroflow.so" -Destination "$PROJECT_DIR\target\android-build\libs\arm64-v8a\libgyroflow_arm64-v8a.so" -Force

Copy-Item -Path "$QT_LIBS\libQt6Widgets_arm64-v8a.so" -Destination "$PROJECT_DIR\target\android-build\libs\arm64-v8a\libQt6Widgets_arm64-v8a.so" -ErrorAction SilentlyContinue
Copy-Item -Path "$QT_LIBS\libQt6Svg_arm64-v8a.so"     -Destination "$PROJECT_DIR\target\android-build\libs\arm64-v8a\libQt6Svg_arm64-v8a.so" -ErrorAction SilentlyContinue
Copy-Item -Path "$QT_LIBS\..\plugins\iconengines\libplugins_iconengines_qsvgicon_arm64-v8a.so" -Destination "$PROJECT_DIR\target\android-build\libs\arm64-v8a\libplugins_iconengines_qsvgicon_arm64-v8a.so" -ErrorAction SilentlyContinue
Copy-Item -Path "$QT_LIBS\..\plugins\imageformats\libplugins_imageformats_qsvg_arm64-v8a.so" -Destination "$PROJECT_DIR\target\android-build\libs\arm64-v8a\libplugins_imageformats_qsvg_arm64-v8a.so" -ErrorAction SilentlyContinue

androiddeployqt --input "$PROJECT_DIR\_deployment\android\android-deploy.json" `
                --output "$PROJECT_DIR\target\android-build" `
                --deployment bundled `
                --android-platform android-30 `
                --jdk ${Env:JAVA_HOME} `
                --gradle

adb install "$PROJECT_DIR\target\android-build\build\outputs\apk\debug\android-build-debug.apk"

# Alternative
# cargo install cargo-ndk
# cargo ndk -t arm64-v8a --platform 26 -o ./jniLibs build --release
