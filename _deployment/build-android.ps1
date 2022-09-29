$PROJECT_DIR="$PSScriptRoot\.."

$BUILD_PROFILE = "deploy"
$QT_LIBS = "$PROJECT_DIR\ext\6.3.1\android_arm64_v8a\lib"
$Env:Path += ";$PROJECT_DIR\ext\6.3.1\android_arm64_v8a\bin"
$Env:Path += ";$PROJECT_DIR\ext\6.3.1\mingw_64\bin\"
$Env:Path += ";$PROJECT_DIR\ext\llvm-13-win64\bin"
$Env:ANDROID_NDK_HOME = "D:\Programy\Android\sdk\ndk\android-ndk-r23c"
$Env:ANDROID_SDK_ROOT = "D:\Programy\Android\sdk\"
$Env:JAVA_HOME = "D:\Programy\Java\jdk-14.0.1"
$Env:QMAKE = "$PROJECT_DIR\ext\6.3.1\android_arm64_v8a\bin\qmake.bat"
$Env:FFMPEG_DIR = "$PROJECT_DIR\ext\ffmpeg-5.1-android-gpl-lite"
$Env:LIBCLANG_PATH = "$PROJECT_DIR\ext\llvm-13-win64\bin"
$Env:OPENCV_LINK_LIBS = "opencv_calib3d,opencv_features2d,opencv_imgproc,opencv_video,opencv_flann,opencv_core,tegra_hal,tbb,ittnotify,z"
$Env:OPENCV_LINK_PATHS = "$PROJECT_DIR\ext\OpenCV-android-sdk\sdk\native\staticlibs\arm64-v8a,$PROJECT_DIR\ext\OpenCV-android-sdk\sdk\native\3rdparty\libs\arm64-v8a"
$Env:OPENCV_INCLUDE_PATHS = "$PROJECT_DIR\ext\OpenCV-android-sdk\sdk\native\jni\include"
$Env:VCPKG_ROOT = "$PROJECT_DIR\ext\vcpkg"

$CLANG_LIB = $Env:LIBCLANG_PATH.replace('\', '/').replace('/bin', '/lib');
$NDK_REPLACED = $Env:ANDROID_NDK_HOME.replace('\', '/');
$SDK_REPLACED = $Env:ANDROID_SDK_ROOT.replace('\', '/');
$PROJECT_DIR_UNIX = $PROJECT_DIR.replace('\', '/');
$Env:BINDGEN_EXTRA_CLANG_ARGS = "-I$CLANG_LIB/clang/13.0.0/include --sysroot=$NDK_REPLACED/toolchains/llvm/prebuilt/windows-x86_64/sysroot"

Copy-Item -Path "$QT_LIBS\libQt6Core_arm64-v8a.so"           -Destination "$QT_LIBS\libQt6Core.so"           -ErrorAction SilentlyContinue
Copy-Item -Path "$QT_LIBS\libQt6Gui_arm64-v8a.so"            -Destination "$QT_LIBS\libQt6Gui.so"            -ErrorAction SilentlyContinue
Copy-Item -Path "$QT_LIBS\libQt6Widgets_arm64-v8a.so"        -Destination "$QT_LIBS\libQt6Widgets.so"        -ErrorAction SilentlyContinue
Copy-Item -Path "$QT_LIBS\libQt6Quick_arm64-v8a.so"          -Destination "$QT_LIBS\libQt6Quick.so"          -ErrorAction SilentlyContinue
Copy-Item -Path "$QT_LIBS\libQt6Qml_arm64-v8a.so"            -Destination "$QT_LIBS\libQt6Qml.so"            -ErrorAction SilentlyContinue
Copy-Item -Path "$QT_LIBS\libQt6QuickControls2_arm64-v8a.so" -Destination "$QT_LIBS\libQt6QuickControls2.so" -ErrorAction SilentlyContinue

# Replace [[bin]] with [lib]
[System.IO.File]::WriteAllText("$PROJECT_DIR\Cargo.toml", [System.IO.File]::ReadAllText("$PROJECT_DIR\Cargo.toml").Replace("[[bin]]", "[lib]`ncrate-type = [""cdylib""]"))

cargo apk build --profile $BUILD_PROFILE

# Restore [[bin]]
[System.IO.File]::WriteAllText("$PROJECT_DIR\Cargo.toml", [System.IO.File]::ReadAllText("$PROJECT_DIR\Cargo.toml").Replace("[lib]`ncrate-type = [""cdylib""]", "[[bin]]"))

mkdir "$PROJECT_DIR\target\android-build" -ErrorAction SilentlyContinue
mkdir "$PROJECT_DIR\target\android-build\libs" -ErrorAction SilentlyContinue
Copy-Item -Path "$PROJECT_DIR\target\$BUILD_PROFILE\apk\lib\*" -Destination "$PROJECT_DIR\target\android-build\libs\" -Recurse -Force
Copy-Item -Path "$PROJECT_DIR\_deployment\android\src" -Destination "$PROJECT_DIR\target\android-build\" -Recurse -Force
# Copy-Item -Path "$PROJECT_DIR\target\aarch64-linux-android\$BUILD_PROFILE\libffmpeg.so" -Destination "$PROJECT_DIR\target\android-build\libs\arm64-v8a\" -Force
# Copy-Item -Path "$PROJECT_DIR\target\aarch64-linux-android\$BUILD_PROFILE\libqtav-mediacodec.so" -Destination "$PROJECT_DIR\target\android-build\libs\arm64-v8a\" -Force
Move-Item -Path "$PROJECT_DIR\target\android-build\libs\arm64-v8a\libgyroflow.so" -Destination "$PROJECT_DIR\target\android-build\libs\arm64-v8a\libgyroflow_arm64-v8a.so" -Force

$qtlibs = @(
    "libQt6LabsFolderListModel_arm64-v8a.so",
    "libQt6LabsSettings_arm64-v8a.so",
    "libQt6QmlLocalStorage_arm64-v8a.so",
    "libQt6QmlWorkerScript_arm64-v8a.so",
    "libQt6QmlXmlListModel_arm64-v8a.so",
    "libQt6QuickControls2_arm64-v8a.so",
    "libQt6QuickControls2Impl_arm64-v8a.so",
    "libQt6QuickDialogs2_arm64-v8a.so",
    "libQt6QuickDialogs2QuickImpl_arm64-v8a.so",
    "libQt6QuickDialogs2Utils_arm64-v8a.so",
    "libQt6QuickLayouts_arm64-v8a.so",
    "libQt6QuickParticles_arm64-v8a.so",
    "libQt6QuickShapes_arm64-v8a.so",
    "libQt6QuickTemplates2_arm64-v8a.so",
    "libQt6Sql_arm64-v8a.so",
    "libQt6Svg_arm64-v8a.so",
    "libQt6Core_arm64-v8a.so",
    "libQt6Gui_arm64-v8a.so",
    "libQt6Network_arm64-v8a.so",
    "libQt6OpenGL_arm64-v8a.so",
    "libQt6Qml_arm64-v8a.so",
    "libQt6QmlModels_arm64-v8a.so",
    "libQt6Quick_arm64-v8a.so",
    "libQt6QuickControls2_arm64-v8a.so",
    "libQt6QuickTemplates2_arm64-v8a.so",
    "libQt6Widgets_arm64-v8a.so",
    "..\plugins\iconengines\libplugins_iconengines_qsvgicon_arm64-v8a.so",
    "..\plugins\imageformats\libplugins_imageformats_qsvg_arm64-v8a.so",
    "..\plugins\sqldrivers\libplugins_sqldrivers_qsqlite_arm64-v8a.so",
    "..\qml\Qt\labs\folderlistmodel\libqml_Qt_labs_folderlistmodel_qmlfolderlistmodelplugin_arm64-v8a.so",
    "..\qml\Qt\labs\settings\libqml_Qt_labs_settings_qmlsettingsplugin_arm64-v8a.so",
    "..\qml\QtQml\libqml_QtQml_qmlplugin_arm64-v8a.so",
    "..\qml\QtQml\Models\libqml_QtQml_Models_modelsplugin_arm64-v8a.so",
    "..\qml\QtQml\WorkerScript\libqml_QtQml_WorkerScript_workerscriptplugin_arm64-v8a.so",
    "..\qml\QtQml\XmlListModel\libqml_QtQml_XmlListModel_qmlxmllistmodelplugin_arm64-v8a.so",
    "..\qml\QtQuick\Controls\Basic\impl\libqml_QtQuick_Controls_Basic_impl_qtquickcontrols2basicstyleimplplugin_arm64-v8a.so",
    "..\qml\QtQuick\Controls\Basic\libqml_QtQuick_Controls_Basic_qtquickcontrols2basicstyleplugin_arm64-v8a.so",
    "..\qml\QtQuick\Controls\impl\libqml_QtQuick_Controls_impl_qtquickcontrols2implplugin_arm64-v8a.so",
    "..\qml\QtQuick\Controls\libqml_QtQuick_Controls_qtquickcontrols2plugin_arm64-v8a.so",
    "..\qml\QtQuick\Controls\Material\impl\libqml_QtQuick_Controls_Material_impl_qtquickcontrols2materialstyleimplplugin_arm64-v8a.so",
    "..\qml\QtQuick\Controls\Material\libqml_QtQuick_Controls_Material_qtquickcontrols2materialstyleplugin_arm64-v8a.so",
    "..\qml\QtQuick\Dialogs\libqml_QtQuick_Dialogs_qtquickdialogsplugin_arm64-v8a.so",
    "..\qml\QtQuick\Dialogs\quickimpl\libqml_QtQuick_Dialogs_quickimpl_qtquickdialogs2quickimplplugin_arm64-v8a.so",
    "..\qml\QtQuick\Layouts\libqml_QtQuick_Layouts_qquicklayoutsplugin_arm64-v8a.so",
    "..\qml\QtQuick\libqml_QtQuick_qtquick2plugin_arm64-v8a.so",
    "..\qml\QtQuick\LocalStorage\libqml_QtQuick_LocalStorage_qmllocalstorageplugin_arm64-v8a.so",
    "..\qml\QtQuick\NativeStyle\libqml_QtQuick_NativeStyle_qtquickcontrols2nativestyleplugin_arm64-v8a.so",
    "..\qml\QtQuick\Particles\libqml_QtQuick_Particles_particlesplugin_arm64-v8a.so",
    "..\qml\QtQuick\Shapes\libqml_QtQuick_Shapes_qmlshapesplugin_arm64-v8a.so",
    "..\qml\QtQuick\Templates\libqml_QtQuick_Templates_qtquicktemplates2plugin_arm64-v8a.so",
    "..\qml\QtQuick\tooling\libqml_QtQuick_tooling_quicktoolingplugin_arm64-v8a.so",
    "..\qml\QtQuick\Window\libqml_QtQuick_Window_quickwindowplugin_arm64-v8a.so"
);
foreach ($x in $qtlibs) {
    Copy-Item -Path "$QT_LIBS\$x" -Destination "$PROJECT_DIR\target\android-build\libs\arm64-v8a\" -ErrorAction SilentlyContinue
}

$androiddeploy = @"
{
   "description": "",
   "qt": "$PROJECT_DIR_UNIX/ext/6.3.1/android_arm64_v8a",
   "sdk": "$SDK_REPLACED",
   "sdkBuildToolsRevision": "30.0.3",
   "ndk": "$NDK_REPLACED",
   "toolchain-prefix": "llvm",
   "tool-prefix": "llvm",
   "ndk-host": "windows-x86_64",
   "architectures": {"arm64-v8a":"aarch64-linux-android"},
   "android-min-sdk-version": "23",
   "android-package-source-directory": "$PROJECT_DIR_UNIX/_deployment/android",
   "android-target-sdk-version": "29",
   "qml-importscanner-binary": "$PROJECT_DIR_UNIX/ext/6.3.1/mingw_64/bin/qmlimportscanner",
   "rcc-binary": "$PROJECT_DIR_UNIX/ext/6.3.1/mingw_64/bin/rcc",
   "qml-root-path": "$PROJECT_DIR_UNIX/src",
   "stdcpp-path": "$NDK_REPLACED/toolchains/llvm/prebuilt/windows-x86_64/sysroot/usr/lib",
   "qrcFiles": "",
   "application-binary": "gyroflow"
}
"@
$androiddeploy | Out-File -encoding utf8 -FilePath "$PROJECT_DIR\target\android-build\android-deploy.json"

androiddeployqt --input "$PROJECT_DIR\target\android-build\android-deploy.json" `
                --output "$PROJECT_DIR\target\android-build" `
                --deployment bundled `
                --android-platform android-30 `
                --jdk ${Env:JAVA_HOME} `
                --gradle

adb install "$PROJECT_DIR\target\android-build\build\outputs\apk\debug\android-build-debug.apk"

# Alternative
# cargo install cargo-ndk
# cargo ndk -t arm64-v8a --platform 26 -o ./jniLibs build --release
