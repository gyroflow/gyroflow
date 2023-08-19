!include common.just

KeyStore         := env_var_or_default("KEY_STORE_PATH",  "")
KeyStoreAlias    := env_var_or_default("KEY_STORE_ALIAS", "")
KeyStorePassword := env_var_or_default("KEY_STORE_PASS",  "")

export ANDROID_NDK_HOME := env_var_or_default("ANDROID_NDK_HOME", "D:\\Programy\\Android\\sdk\\ndk\\android-ndk-r23c")
export ANDROID_SDK_ROOT := env_var_or_default("ANDROID_SDK_ROOT", "D:\\Programy\\Android\\sdk\\")
export JAVA_HOME        := env_var_or_default("JAVA_HOME",        "D:\\Programy\\Java\\jdk-14.0.1")
export BUILD_PROFILE    := env_var_or_default("BUILD_PROFILE",    "release") # use "deploy" for final apk

# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ Dependencies ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

[no-cd]
install-deps:
    #!powershell
    $ProgressPreference = 'SilentlyContinue'
    $ErrorActionPreference = 'Stop'
    mkdir "{{ExtDir}}" -ErrorAction SilentlyContinue
    cd {{ExtDir}}

    $Python = "python";
    if (Get-Command "python.exe" -ErrorAction SilentlyContinue) {
        $Python = "python";
    } elseif (Get-Command "python3.exe" -ErrorAction SilentlyContinue) {
        $Python = "python3";
    } else {
        if (-not (Test-Path -Path "{{ExtDir}}/WPy64-310100b3")) {
            echo "Downloading WinPython"
            wget https://github.com/winpython/winpython/releases/download/6.1.20230227/Winpython64-3.10.10.0dotb3.exe -outfile "winpython.exe"
            7z x -y winpython.exe
            del winpython.exe
        }
        $Python = resolve-path "{{ExtDir}}\WPy64-310100b3\python-3.10.10.amd64\python.exe"
    }

    cargo install --git https://github.com/rust-mobile/cargo-apk.git cargo-apk
    rustup target add aarch64-linux-android

    # ――――――――――――――――――――――――――――――――――――――――― Qt ――――――――――――――――――――――――――――――――――――――――――
    if (-not (Test-Path -Path "{{QtVersion}}/android_arm64_v8a")) {
        echo "Downloading Qt {{QtVersion}} for Android"
        & $Python -m pip install -U pip aqtinstall
        & $Python -m aqt install-qt windows desktop {{QtVersion}} win64_mingw
        & $Python -m aqt install-qt windows android {{QtVersion}} android_arm64_v8a
    }
    # ――――――――――――――――――――――――――――――――――――――――― Qt ――――――――――――――――――――――――――――――――――――――――――

    # ――――――――――――――――――――――――――――――――――――――― FFmpeg ――――――――――――――――――――――――――――――――――――――――
    $FFmpeg = "ffmpeg-master-android-gpl-lite"
    if (-not (Test-Path -Path "$FFmpeg")) {
        echo "Downloading $FFmpeg"
        wget "https://master.dl.sourceforge.net/project/avbuild/android/$FFmpeg.tar.xz?viasf=1" -outfile "ffmpeg.tar.xz"
        7z x -aoa ffmpeg.tar.xz
        del ffmpeg.tar.xz
        7z x -aoa ffmpeg.tar
        del ffmpeg.tar
    }
    # ――――――――――――――――――――――――――――――――――――――― FFmpeg ――――――――――――――――――――――――――――――――――――――――

    # ――――――――――――――――――――――――――――――――――――――― OpenCV ――――――――――――――――――――――――――――――――――――――――
    if (-not (Test-Path -Path "OpenCV-android-sdk/")) {
        echo "Downloading OpenCV for Android"
        wget "https://github.com/opencv/opencv/releases/download/4.7.0/opencv-4.7.0-android-sdk.zip" -outfile "opencv.zip"
        7z x -aoa opencv.zip
        del opencv.zip
    }
    # ――――――――――――――――――――――――――――――――――――――― OpenCV ――――――――――――――――――――――――――――――――――――――――

    # ――――――――――――――――――――――――――――――――――――――― LLVM ――――――――――――――――――――――――――――――――――――――――
    if (-not (Test-Path -Path "{{LLVMPath}}\libclang.dll")) {
        echo "Downloading LLVM"
        wget "https://github.com/llvm/llvm-project/releases/download/llvmorg-16.0.0/LLVM-16.0.0-win64.exe" -outfile "llvm-win64.exe"
        7z x -y llvm-win64.exe -ollvm
        del "llvm-win64.exe"
    }
    # ――――――――――――――――――――――――――――――――――――――― LLVM ――――――――――――――――――――――――――――――――――――――――

# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ Development ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

#[no-cd]
#run *param:
#    cargo run --release -- {{param}}
#
#[no-cd]
#debug *param:
#    cargo run -- {{param}}

# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ Deployment ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

[no-cd]
deploy *param:
    #!powershell
    $ErrorActionPreference = 'Stop'

    $ProjectDir = (resolve-path "{{ProjectDir}}").ToString()
    cd $ProjectDir

    $Env:CARGO_TARGET_DIR = "$ProjectDir\target"

    $QT_LIBS = "$ProjectDir\ext\{{QtVersion}}\android_arm64_v8a\lib"
    $Env:Path = "$ProjectDir\ext\{{QtVersion}}\android_arm64_v8a\bin;$ProjectDir\ext\{{QtVersion}}\mingw_64\bin\;$Env:Path"
    $Env:QMAKE = "$ProjectDir\ext\{{QtVersion}}\android_arm64_v8a\bin\qmake.bat"
    $Env:FFMPEG_DIR = "$ProjectDir\ext\ffmpeg-master-android-gpl-lite"
    $Env:OPENCV_LINK_LIBS = "opencv_stitching,opencv_calib3d,opencv_features2d,opencv_imgproc,opencv_video,opencv_flann,opencv_core,tegra_hal,tbb,ittnotify,z"
    $Env:OPENCV_LINK_PATHS = "$ProjectDir\ext\OpenCV-android-sdk\sdk\native\staticlibs\arm64-v8a,$ProjectDir\ext\OpenCV-android-sdk\sdk\native\3rdparty\libs\arm64-v8a"
    $Env:OPENCV_INCLUDE_PATHS = "$ProjectDir\ext\OpenCV-android-sdk\sdk\native\jni\include"

    $CLANG_LIB = $Env:LIBCLANG_PATH.replace('\', '/').replace('/bin', '/lib');
    $NDK_REPLACED = $Env:ANDROID_NDK_HOME.replace('\', '/');
    $SDK_REPLACED = $Env:ANDROID_SDK_ROOT.replace('\', '/');
    $PROJECT_DIR_UNIX = $ProjectDir.replace('\', '/');
    $Env:BINDGEN_EXTRA_CLANG_ARGS = "-I$CLANG_LIB/clang/16.0.0/include --sysroot=$NDK_REPLACED/toolchains/llvm/prebuilt/windows-x86_64/sysroot"
    echo $Env:BINDGEN_EXTRA_CLANG_ARGS

    Copy-Item -Path "$QT_LIBS\libQt6Core_arm64-v8a.so"           -Destination "$QT_LIBS\libQt6Core.so"           -ErrorAction SilentlyContinue
    Copy-Item -Path "$QT_LIBS\libQt6Gui_arm64-v8a.so"            -Destination "$QT_LIBS\libQt6Gui.so"            -ErrorAction SilentlyContinue
    Copy-Item -Path "$QT_LIBS\libQt6Widgets_arm64-v8a.so"        -Destination "$QT_LIBS\libQt6Widgets.so"        -ErrorAction SilentlyContinue
    Copy-Item -Path "$QT_LIBS\libQt6Quick_arm64-v8a.so"          -Destination "$QT_LIBS\libQt6Quick.so"          -ErrorAction SilentlyContinue
    Copy-Item -Path "$QT_LIBS\libQt6Qml_arm64-v8a.so"            -Destination "$QT_LIBS\libQt6Qml.so"            -ErrorAction SilentlyContinue
    Copy-Item -Path "$QT_LIBS\libQt6QuickControls2_arm64-v8a.so" -Destination "$QT_LIBS\libQt6QuickControls2.so" -ErrorAction SilentlyContinue

    # Replace [[bin]] with [lib]
    [System.IO.File]::WriteAllText("$ProjectDir\Cargo.toml", [System.IO.File]::ReadAllText("$ProjectDir\Cargo.toml").Replace("[[bin]]", "[lib]`ncrate-type = [""cdylib""]"))

    cargo apk build --profile $Env:BUILD_PROFILE

    # Restore [[bin]]
    [System.IO.File]::WriteAllText("$ProjectDir\Cargo.toml", [System.IO.File]::ReadAllText("$ProjectDir\Cargo.toml").Replace("[lib]`ncrate-type = [""cdylib""]", "[[bin]]"))

    mkdir "$ProjectDir\target\android-build" -ErrorAction SilentlyContinue
    mkdir "$ProjectDir\target\android-build\libs" -ErrorAction SilentlyContinue
    Copy-Item -Path "$ProjectDir\target\$Env:BUILD_PROFILE\apk\lib\*" -Destination "$ProjectDir\target\android-build\libs\" -Recurse -Force
    Copy-Item -Path "$ProjectDir\_deployment\android\src" -Destination "$ProjectDir\target\android-build\" -Recurse -Force
    Copy-Item -Path "$ProjectDir\target\aarch64-linux-android\$Env:BUILD_PROFILE\libffmpeg.so" -Destination "$ProjectDir\target\android-build\libs\arm64-v8a\" -Force
    # Copy-Item -Path "$ProjectDir\target\aarch64-linux-android\$Env:BUILD_PROFILE\libqtav-mediacodec.so" -Destination "$ProjectDir\target\android-build\libs\arm64-v8a\" -Force
    Move-Item -Path "$ProjectDir\target\android-build\libs\arm64-v8a\libgyroflow.so" -Destination "$ProjectDir\target\android-build\libs\arm64-v8a\libgyroflow_arm64-v8a.so" -Force

    $qtlibs = @(
        "libQt6Core_arm64-v8a.so",
        "libQt6Gui_arm64-v8a.so",
        "libQt6LabsFolderListModel_arm64-v8a.so",
        "libQt6LabsSettings_arm64-v8a.so",
        "libQt6Network_arm64-v8a.so",
        "libQt6OpenGL_arm64-v8a.so",
        "libQt6Qml_arm64-v8a.so",
        "libQt6QmlLocalStorage_arm64-v8a.so",
        "libQt6QmlModels_arm64-v8a.so",
        "libQt6QmlWorkerScript_arm64-v8a.so",
        "libQt6QmlXmlListModel_arm64-v8a.so",
        "libQt6Quick_arm64-v8a.so",
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
        "libQt6Widgets_arm64-v8a.so",
        "..\plugins\iconengines\libplugins_iconengines_qsvgicon_arm64-v8a.so",
        "..\plugins\imageformats\libplugins_imageformats_qsvg_arm64-v8a.so",
        "..\plugins\sqldrivers\libplugins_sqldrivers_qsqlite_arm64-v8a.so",
        "..\qml\Qt\labs\folderlistmodel\libqml_Qt_labs_folderlistmodel_qmlfolderlistmodelplugin_arm64-v8a.so",
        "..\qml\Qt\labs\settings\libqml_Qt_labs_settings_qmlsettingsplugin_arm64-v8a.so",
        "..\qml\QtCore\libqml_QtCore_qtqmlcoreplugin_arm64-v8a.so",
        "..\qml\QtQml\Base\libqml_QtQml_Base_qmlplugin_arm64-v8a.so",
        "..\qml\QtQml\libqml_QtQml_qmlmetaplugin_arm64-v8a.so",
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
        Copy-Item -Path "$QT_LIBS\$x" -Destination "$ProjectDir\target\android-build\libs\arm64-v8a\" -Force -ErrorAction SilentlyContinue
    }

    # Strip all libs
    . "$Env:ANDROID_NDK_HOME\toolchains\llvm\prebuilt\windows-x86_64\bin\llvm-strip.exe" "$ProjectDir\target\android-build\libs\arm64-v8a\*.so"

    $androiddeploy = @"
    {
        "description": "",
        "qt": "$PROJECT_DIR_UNIX/ext/{{QtVersion}}/android_arm64_v8a",
        "qtDataDirectory": ".",
        "qtLibExecsDirectory": "libexec",
        "qtLibsDirectory": "lib",
        "qtPluginsDirectory": "plugins",
        "qtQmlDirectory": "qml",
        "sdk": "$SDK_REPLACED",
        "sdkBuildToolsRevision": "30.0.3",
        "ndk": "$NDK_REPLACED",
        "toolchain-prefix": "llvm",
        "tool-prefix": "llvm",
        "ndk-host": "windows-x86_64",
        "architectures": {"arm64-v8a":"aarch64-linux-android"},
        "android-min-sdk-version": "26",
        "android-package-source-directory": "$PROJECT_DIR_UNIX/_deployment/android",
        "android-target-sdk-version": "33",
        "qml-importscanner-binary": "$PROJECT_DIR_UNIX/ext/{{QtVersion}}/mingw_64/bin/qmlimportscanner.exe",
        "rcc-binary": "$PROJECT_DIR_UNIX/ext/{{QtVersion}}/mingw_64/bin/rcc.exe",
        "qml-root-path": "$PROJECT_DIR_UNIX/src",
        "stdcpp-path": "$NDK_REPLACED/toolchains/llvm/prebuilt/windows-x86_64/sysroot/usr/lib",
        "qrcFiles": "",
        "application-binary": "gyroflow"
    }
    "@
    $androiddeploy | Out-File -encoding utf8 -FilePath "$ProjectDir\target\android-build\android-deploy.json"

    $job = Start-Job -Name "Cleanup" -ArgumentList "$ProjectDir\target\android-build\libs\arm64-v8a" -ScriptBlock {
        for ($num = 1; $num -le 400 ; $num++) {
            Remove-Item "${args}\*Universal*"
            Remove-Item "${args}\*Imagine*"
            Remove-Item "${args}\*Fusion*"
            Remove-Item "${args}\*Particles*"
            Remove-Item "${args}\*tooling*"
            Remove-Item "${args}\*qgif*"
            Remove-Item "${args}\*qico*"
            Start-Sleep -Milliseconds 150;
        }
    }
    Get-job | Receive-Job

    $Sign = ""
    $ApkName = "apk\debug\android-build-debug.apk"
    if ("{{KeyStore}}" -and "{{KeyStoreAlias}}" -and "{{KeyStorePassword}}") {
        $Sign = "--release --sign {{KeyStore}} {{KeyStoreAlias}} --storepass {{KeyStorePassword}}"
        $ApkName = "apk\release\android-build-release-signed.apk"
    }

    $cmd = "androiddeployqt --input `"$ProjectDir\target\android-build\android-deploy.json`" --output `"$ProjectDir\target\android-build`" --deployment bundled $Sign --android-platform android-33 --jdk ${Env:JAVA_HOME} --gradle"

    iex "$cmd"
    if ("$Env:BUILD_APP_STORE_BUNDLE" -eq "true") {
        iex "$cmd --aab"
    }

    Stop-Job $job

    New-Item -Type dir "$ProjectDir\_deployment\_binaries\" -ErrorAction SilentlyContinue
    Copy-Item -Path "$ProjectDir\target\android-build\build\outputs\$ApkName" -Destination "$ProjectDir\_deployment\_binaries\Gyroflow.apk" -Force -ErrorAction SilentlyContinue
    Copy-Item -Path "$ProjectDir\target\android-build\build\outputs\bundle\release\android-build-release.aab" -Destination "$ProjectDir\_deployment\_binaries\Gyroflow.aab" -Force -ErrorAction SilentlyContinue

    if ("{{param}}" -eq "install") {
        adb install "$ProjectDir\_deployment\_binaries\Gyroflow.apk"
    }
