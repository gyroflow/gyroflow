!include common.just

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

    # ――――――――――――――――――――――――――――――――――――――――― Qt ――――――――――――――――――――――――――――――――――――――――――
    if (-not (Test-Path -Path "{{QtVersion}}/msvc2019_64")) {
        echo "Downloading Qt {{QtVersion}}"
        & $Python -m pip install -U pip aqtinstall
        & $Python -m aqt install-qt windows desktop {{QtVersion}} win64_msvc2019_64 -m qtshadertools
    }
    # ――――――――――――――――――――――――――――――――――――――――― Qt ――――――――――――――――――――――――――――――――――――――――――

    # ――――――――――――――――――――――――――――――――――――――― FFmpeg ――――――――――――――――――――――――――――――――――――――――
    if (-not (Test-Path -Path "{{FFmpeg}}")) {
        echo "Downloading {{FFmpeg}}"
        wget "https://master.dl.sourceforge.net/project/avbuild/windows-desktop/{{FFmpeg}}.7z?viasf=1" -outfile "{{FFmpeg}}.7z"
        7z x -y {{FFmpeg}}.7z
        del {{FFmpeg}}.7z
    }
    # ――――――――――――――――――――――――――――――――――――――― FFmpeg ――――――――――――――――――――――――――――――――――――――――

    # ――――――――――――――――――――――――――――――――――――――― OpenCV ――――――――――――――――――――――――――――――――――――――――
    if (-not (Test-Path -Path "vcpkg/installed/{{VcpkgArch}}/lib/opencv_core4.lib")) {
        echo "Downloading OpenCV"
        rm -Recurse -Force \vcpkg -ErrorAction SilentlyContinue
        git clone --depth 1 https://github.com/Microsoft/vcpkg.git
        .\vcpkg\bootstrap-vcpkg.bat -disableMetrics
        .\vcpkg\vcpkg install "opencv4[core]:{{VcpkgArch}}"
        .\vcpkg\vcpkg install "opencl:{{VcpkgArch}}"
        rm -Recurse -Force .\vcpkg\buildtrees
        rm -Recurse -Force .\vcpkg\downloads
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

[no-cd]
build *param:
    cargo build --release -- {{param}}

[no-cd]
build-debug *param:
    $Env:OPENCV_LINK_LIBS="$Env:OPENCV_LINK_LIBS,opencv_ml4,opencv_highgui4,opencv_videoio4" ; \
    cargo build -- {{param}}

[no-cd]
run *param:
    cargo run --release -- {{param}}

[no-cd]
clippy:
    cargo clippy

[no-cd]
debug *param:
    $Env:OPENCV_LINK_LIBS="$Env:OPENCV_LINK_LIBS,opencv_ml4,opencv_highgui4,opencv_videoio4" ; \
    cargo run -- {{param}}

[no-cd]
profile *param:
    cargo run --profile profile -- {{param}}

# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ Deployment ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

[no-cd]
deploy:
    #!powershell
    $ErrorActionPreference = 'Stop'

    $ProjectDir = resolve-path "{{ProjectDir}}"
    cd $ProjectDir
    mkdir "$ProjectDir\_deployment\_binaries\win64" -Force

    cargo build --profile deploy

    $TargetDir   = resolve-path "{{ProjectDir}}\_deployment\_binaries\win64"
    $QtDir       = resolve-path "{{QtPath}}\.."
    $CargoTarget = resolve-path "{{ProjectDir}}\target\deploy"
    $FFmpegDir   = resolve-path "{{FFMPEG_DIR}}"
    $OpenCVDir   = resolve-path "{{OpenCVPath}}"

    # Copy Qt
    xcopy /Y "$QtDir\plugins\platforms\qwindows.dll"                                                 "$TargetDir\platforms\"
    xcopy /Y "$QtDir\plugins\iconengines\qsvgicon.dll"                                               "$TargetDir\iconengines\"
    xcopy /Y "$QtDir\plugins\imageformats\qsvg.dll"                                                  "$TargetDir\imageformats\"
    xcopy /Y "$QtDir\plugins\imageformats\qjpeg.dll"                                                 "$TargetDir\imageformats\"
    xcopy /Y "$QtDir\bin\Qt6Core.dll"                                                                "$TargetDir\"
    xcopy /Y "$QtDir\bin\Qt6Gui.dll"                                                                 "$TargetDir\"
    xcopy /Y "$QtDir\bin\Qt6Network.dll"                                                             "$TargetDir\"
    xcopy /Y "$QtDir\bin\Qt6OpenGL.dll"                                                              "$TargetDir\"
    xcopy /Y "$QtDir\bin\Qt6Qml.dll"                                                                 "$TargetDir\"
    xcopy /Y "$QtDir\bin\Qt6QmlCore.dll"                                                             "$TargetDir\"
    xcopy /Y "$QtDir\bin\Qt6QmlModels.dll"                                                           "$TargetDir\"
    xcopy /Y "$QtDir\bin\Qt6QmlWorkerScript.dll"                                                     "$TargetDir\"
    xcopy /Y "$QtDir\bin\Qt6Quick.dll"                                                               "$TargetDir\"
    xcopy /Y "$QtDir\bin\Qt6QuickControls2.dll"                                                      "$TargetDir\"
    xcopy /Y "$QtDir\bin\Qt6QuickControls2Impl.dll"                                                  "$TargetDir\"
    xcopy /Y "$QtDir\bin\Qt6QuickDialogs2.dll"                                                       "$TargetDir\"
    xcopy /Y "$QtDir\bin\Qt6QuickDialogs2QuickImpl.dll"                                              "$TargetDir\"
    xcopy /Y "$QtDir\bin\Qt6QuickDialogs2Utils.dll"                                                  "$TargetDir\"
    xcopy /Y "$QtDir\bin\Qt6QuickEffects.dll"                                                        "$TargetDir\"
    xcopy /Y "$QtDir\bin\Qt6QuickTemplates2.dll"                                                     "$TargetDir\"
    xcopy /Y "$QtDir\bin\Qt6QuickLayouts.dll"                                                        "$TargetDir\"
    xcopy /Y "$QtDir\bin\Qt6Svg.dll"                                                                 "$TargetDir\"

    # Remove once moved to Qt 6.6.1
    xcopy /Y "$QtDir\bin\Qt6LabsSettings.dll"                                                        "$TargetDir\"
    xcopy /Y "$QtDir\qml\Qt\labs\settings\qmldir"                                                    "$TargetDir\Qt\labs\settings\"
    xcopy /Y "$QtDir\qml\Qt\labs\settings\qmlsettingsplugin.dll"                                     "$TargetDir\Qt\labs\settings\"

    # Copy QtQuick
    xcopy /Y "$QtDir\qml\QtCore\qmldir"                                                              "$TargetDir\QtCore\"
    xcopy /Y "$QtDir\qml\QtCore\qtqmlcoreplugin.dll"                                                 "$TargetDir\QtCore\"
    xcopy /Y "$QtDir\qml\QtQml\qmldir"                                                               "$TargetDir\QtQml\"
    xcopy /Y "$QtDir\qml\QtQml\qmlplugin.dll"                                                        "$TargetDir\QtQml\"
    xcopy /Y "$QtDir\qml\QtQml\qmlmetaplugin.dll"                                                    "$TargetDir\QtQml\"
    xcopy /Y "$QtDir\qml\QtQml\Base\qmldir"                                                          "$TargetDir\QtQml\Base\"
    xcopy /Y "$QtDir\qml\QtQml\Base\qmlplugin.dll"                                                   "$TargetDir\QtQml\Base\"
    xcopy /Y "$QtDir\qml\QtQml\Models\qmldir"                                                        "$TargetDir\QtQml\Models\"
    xcopy /Y "$QtDir\qml\QtQml\Models\modelsplugin.dll"                                              "$TargetDir\QtQml\Models\"
    xcopy /Y "$QtDir\qml\QtQml\WorkerScript\qmldir"                                                  "$TargetDir\QtQml\WorkerScript\"
    xcopy /Y "$QtDir\qml\QtQml\WorkerScript\workerscriptplugin.dll"                                  "$TargetDir\QtQml\WorkerScript\"
    xcopy /Y "$QtDir\qml\QtQuick\Controls\Basic\*.qml"                                               "$TargetDir\QtQuick\Controls\Basic\"
    xcopy /Y "$QtDir\qml\QtQuick\Controls\Basic\qmldir"                                              "$TargetDir\QtQuick\Controls\Basic\"
    xcopy /Y "$QtDir\qml\QtQuick\Controls\Basic\qtquickcontrols2basicstyleplugin.dll"                "$TargetDir\QtQuick\Controls\Basic\"
    xcopy /Y "$QtDir\qml\QtQuick\Controls\Basic\impl\qmldir"                                         "$TargetDir\QtQuick\Controls\Basic\impl\"
    xcopy /Y "$QtDir\qml\QtQuick\Controls\Basic\impl\qtquickcontrols2basicstyleimplplugin.dll"       "$TargetDir\QtQuick\Controls\Basic\impl\"
    xcopy /Y "$QtDir\qml\QtQuick\Controls\impl\qmldir"                                               "$TargetDir\QtQuick\Controls\impl\"
    xcopy /Y "$QtDir\qml\QtQuick\Controls\impl\qtquickcontrols2implplugin.dll"                       "$TargetDir\QtQuick\Controls\impl\"
    xcopy /Y "$QtDir\qml\QtQuick\Controls\Material\*.qml"                                            "$TargetDir\QtQuick\Controls\Material\"
    xcopy /Y "$QtDir\qml\QtQuick\Controls\Material\qmldir"                                           "$TargetDir\QtQuick\Controls\Material\"
    xcopy /Y "$QtDir\qml\QtQuick\Controls\Material\qtquickcontrols2materialstyleplugin.dll"          "$TargetDir\QtQuick\Controls\Material\"
    xcopy /Y "$QtDir\qml\QtQuick\Controls\Material\impl\*.qml"                                       "$TargetDir\QtQuick\Controls\Material\impl\"
    xcopy /Y "$QtDir\qml\QtQuick\Controls\Material\impl\qmldir"                                      "$TargetDir\QtQuick\Controls\Material\impl\"
    xcopy /Y "$QtDir\qml\QtQuick\Controls\Material\impl\qtquickcontrols2materialstyleimplplugin.dll" "$TargetDir\QtQuick\Controls\Material\impl\"
    xcopy /Y "$QtDir\qml\QtQuick\Controls\qmldir"                                                    "$TargetDir\QtQuick\Controls\"
    xcopy /Y "$QtDir\qml\QtQuick\Controls\qtquickcontrols2plugin.dll"                                "$TargetDir\QtQuick\Controls\"
    xcopy /Y "$QtDir\qml\QtQuick\Dialogs\qmldir"                                                     "$TargetDir\QtQuick\Dialogs\"
    xcopy /Y "$QtDir\qml\QtQuick\Dialogs\qtquickdialogsplugin.dll"                                   "$TargetDir\QtQuick\Dialogs\"
    xcopy /Y "$QtDir\qml\QtQuick\Dialogs\quickimpl\qml\+Material\*.qml"                              "$TargetDir\QtQuick\Dialogs\quickimpl\qml\+Material\"
    xcopy /Y "$QtDir\qml\QtQuick\Dialogs\quickimpl\qml\*.qml"                                        "$TargetDir\QtQuick\Dialogs\quickimpl\qml\"
    xcopy /Y "$QtDir\qml\QtQuick\Dialogs\quickimpl\qmldir"                                           "$TargetDir\QtQuick\Dialogs\quickimpl\"
    xcopy /Y "$QtDir\qml\QtQuick\Dialogs\quickimpl\qtquickdialogs2quickimplplugin.dll"               "$TargetDir\QtQuick\Dialogs\quickimpl\"
    xcopy /Y "$QtDir\qml\QtQuick\Effects\effectsplugin.dll"                                          "$TargetDir\QtQuick\Effects\"
    xcopy /Y "$QtDir\qml\QtQuick\Effects\qmldir"                                                     "$TargetDir\QtQuick\Effects\"
    xcopy /Y "$QtDir\qml\QtQuick\Layouts\qmldir"                                                     "$TargetDir\QtQuick\Layouts\"
    xcopy /Y "$QtDir\qml\QtQuick\Layouts\qquicklayoutsplugin.dll"                                    "$TargetDir\QtQuick\Layouts\"
    xcopy /Y "$QtDir\qml\QtQuick\qmldir"                                                             "$TargetDir\QtQuick\"
    xcopy /Y "$QtDir\qml\QtQuick\qtquick2plugin.dll"                                                 "$TargetDir\QtQuick\"
    xcopy /Y "$QtDir\qml\QtQuick\Templates\qmldir"                                                   "$TargetDir\QtQuick\Templates\"
    xcopy /Y "$QtDir\qml\QtQuick\Templates\qtquicktemplates2plugin.dll"                              "$TargetDir\QtQuick\Templates\"
    xcopy /Y "$QtDir\qml\QtQuick\Window\qmldir"                                                      "$TargetDir\QtQuick\Window\"
    xcopy /Y "$QtDir\qml\QtQuick\Window\quickwindowplugin.dll"                                       "$TargetDir\QtQuick\Window\"

    # Copy ffmpeg
    xcopy /Y "$FFmpegDir\bin\avcodec-*.dll"     "$TargetDir\"
    xcopy /Y "$FFmpegDir\bin\avdevice-*.dll"    "$TargetDir\"
    xcopy /Y "$FFmpegDir\bin\avfilter-*.dll"    "$TargetDir\"
    xcopy /Y "$FFmpegDir\bin\avformat-*.dll"    "$TargetDir\"
    xcopy /Y "$FFmpegDir\bin\avutil-*.dll"      "$TargetDir\"
    xcopy /Y "$FFmpegDir\bin\swresample-*.dll"  "$TargetDir\"
    xcopy /Y "$FFmpegDir\bin\swscale-*.dll"     "$TargetDir\"
    # xcopy /Y "$FFmpegDir\bin\postproc-*.dll"  "$TargetDir\"

    # Copy OpenCV
    xcopy /Y "$OpenCVDir\opencv_calib*.dll"      "$TargetDir\"
    xcopy /Y "$OpenCVDir\opencv_cor*.dll"        "$TargetDir\"
    xcopy /Y "$OpenCVDir\opencv_features*.dll"   "$TargetDir\"
    xcopy /Y "$OpenCVDir\opencv_flan*.dll"       "$TargetDir\"
    xcopy /Y "$OpenCVDir\opencv_imgpro*.dll"     "$TargetDir\"
    xcopy /Y "$OpenCVDir\opencv_vide*.dll"       "$TargetDir\"
    xcopy /Y "$OpenCVDir\zlib*.dll"              "$TargetDir\"
    del "$TargetDir\opencv_*videoio*"

    # Copy Gyroflow
    xcopy /Y "$CargoTarget\mdk.dll"                                      "$TargetDir\"
    xcopy /Y "$CargoTarget\mdk-braw.dll"                                 "$TargetDir\"
    xcopy /Y "$CargoTarget\mdk-r3d.dll"                                  "$TargetDir\"
    echo F | xcopy /Y "$CargoTarget\gyroflow.exe"                        "$TargetDir\Gyroflow.exe"
    xcopy /Y "$ProjectDir\_deployment\windows\Gyroflow_with_console.bat" "$TargetDir\"
    xcopy /Y /E "$ProjectDir\resources\camera_presets\*"                 "$TargetDir\camera_presets\"

    # Other
    xcopy /Y "$QtDir\bin\d3dcompiler_47.dll"                             "$TargetDir\"
    xcopy /Y "C:\Program Files (x86)\Windows Kits\10\bin\10.0.22621.0\x64\d3dcompiler_47.dll" "$TargetDir\"

    7z a $ProjectDir\_deployment\_binaries\Gyroflow-windows64.zip $TargetDir\*

[no-cd]
bundle:
    #!powershell
    $ProgressPreference = 'SilentlyContinue'
    $ErrorActionPreference = 'Stop'
    function Invoke-Environment { param([Parameter(Mandatory=$true)] [string] $Command)
        $Command = "`"" + $Command + "`""
        cmd /c "$Command > nul 2>&1 && set" | . { process {
            if ($_ -match '^([^=]+)=(.*)') { [System.Environment]::SetEnvironmentVariable($matches[1], $matches[2]) }
        }}
    }

    $TargetDir  = resolve-path "{{ProjectDir}}\_deployment\_binaries\win64"
    $ProjectDir = resolve-path "{{ProjectDir}}"
    cd $ProjectDir\_deployment\windows\

    if (-Not (Get-Command "makeappx.exe" -ErrorAction SilentlyContinue)) {
        $VSPath = "`"${Env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe`" -latest -property installationPath"
        $VSPath = iex "& $VSPath"
        Invoke-Environment "$VSPath\VC\Auxiliary\Build\vcvars64.bat"
    }

    # Bundle BRAW and RED SDK
    if (-not (Test-Path -Path "$TargetDir\REDR3D-x64.dll")) {
        Invoke-WebRequest -URI "https://api.gyroflow.xyz/sdk/RED_SDK_Windows_8.4.0.tar.gz" -OutFile tmp.tar.gz
        7z x tmp.tar.gz
        7z x -o"$TargetDir\" tmp.tar
        Remove-Item -Path tmp.ta* -Force
    }
    if (-not (Test-Path -Path "$TargetDir\BlackmagicRawAPI.dll")) {
        Invoke-WebRequest -URI "https://api.gyroflow.xyz/sdk/Blackmagic_RAW_SDK_Windows_3.4.tar.gz" -OutFile tmp.tar.gz
        7z x tmp.tar.gz
        7z x -o"$TargetDir\" tmp.tar
        Remove-Item -Path tmp.ta* -Force
    }

    # Generate self-signed key
    # $cert = New-SelfSignedCertificate -Subject "CN=10F1B828-8298-48D5-8164-662492DBDECB" -DnsName 29160AdrianRoss.Gyroflow -Type CodeSigning -CertStoreLocation Cert:\CurrentUser\My
    # $CertPassword = ConvertTo-SecureString -String "$Env:MS_STORE_SIGNING_PASS" -Force -AsPlainText
    # Export-PfxCertificate -Cert "cert:\CurrentUser\My\$($cert.Thumbprint)" -FilePath "msix_signing_key.pfx" -Password $CertPassword

    # Build .msi installer
    # msbuild.exe /p:Configuration=Release -Restore
    # MsixPackagingTool.exe create-package --template installer_msix.xml

    # Build .msix
    Remove-Item -Path "..\_binaries\Gyroflow.msix" -ErrorAction SilentlyContinue
    mkdir -Force "bundle"
    cp -Recurse -Force $TargetDir\* bundle\
    cp -Recurse -Force Assets bundle\Assets
    cp -Force AppxManifest.xml bundle\

    # Apps are not allowed to have a Version with a revision number other than zero specified in the app manifest.
    if ("$Env:GITHUB_RUN_NUMBER") {
        $VersionNoDots = "{{ProjectVersion}}" -replace "[^0-9\.]", '' -replace "\.([0-9]+)$", '$1'
        (Get-Content bundle\AppxManifest.xml) -replace 'Identity Version="[0-9\.]+"', "Identity Version=`"$VersionNoDots.$Env:GITHUB_RUN_NUMBER.0`"" | Out-File bundle\AppxManifest.xml -Encoding Utf8
    }

    makepri.exe createconfig /cf priconfig.xml /dq en-us
    makepri.exe new /pr bundle\Assets\ /cf priconfig.xml /mn bundle\AppxManifest.xml /of bundle\resources.pri /o

    makeappx.exe pack /d bundle /p ..\_binaries\Gyroflow.msix
    signtool.exe sign /f msix_signing_key.pfx /p $Env:MS_STORE_SIGNING_PASS /fd sha256 ..\_binaries\Gyroflow.msix

    Remove-Item -Path "priconfig.xml" -ErrorAction SilentlyContinue
    Remove-Item -Path "bundle" -Recurse -ErrorAction SilentlyContinue
