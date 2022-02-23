@echo off
if "%1"=="" ( set "PROJECT_DIR=D:\programowanie\projekty\Rust\gyroflow" ) else ( set "PROJECT_DIR=%1" )
if "%2"=="" ( set "QT_DIR=D:\Programy\Qt\6.2.3\msvc2019_64" ) else ( set "QT_DIR=%2" )
if "%3"=="" ( set "OPENCV_DIR=%PROJECT_DIR%\ext\opencv-4.5.4\bin" ) else ( set "OPENCV_DIR=%3" )
if "%4"=="" ( set "CARGO_TARGET=%PROJECT_DIR%\target\deploy" ) else ( set "CARGO_TARGET=%4" )
if "%FFMPEG_DIR%"=="" ( set FFMPEG_DIR=%PROJECT_DIR%\ext\ffmpeg-5.0-windows-desktop-clang-gpl-lite )

set TARGET=%PROJECT_DIR%\_deployment\_binaries\win64

:: Copy Qt
xcopy /Y "%QT_DIR%\plugins\platforms\qwindows.dll"                                                 "%TARGET%\platforms\"
xcopy /Y "%QT_DIR%\plugins\iconengines\qsvgicon.dll"                                               "%TARGET%\iconengines\"
xcopy /Y "%QT_DIR%\plugins\imageformats\qsvg.dll"                                                  "%TARGET%\imageformats\"
xcopy /Y "%QT_DIR%\bin\Qt6Core.dll"                                                                "%TARGET%\"
xcopy /Y "%QT_DIR%\bin\Qt6Gui.dll"                                                                 "%TARGET%\"
xcopy /Y "%QT_DIR%\bin\Qt6LabsSettings.dll"                                                        "%TARGET%\"
xcopy /Y "%QT_DIR%\bin\Qt6Network.dll"                                                             "%TARGET%\"
xcopy /Y "%QT_DIR%\bin\Qt6OpenGL.dll"                                                              "%TARGET%\"
xcopy /Y "%QT_DIR%\bin\Qt6Qml.dll"                                                                 "%TARGET%\"
xcopy /Y "%QT_DIR%\bin\Qt6QmlModels.dll"                                                           "%TARGET%\"
xcopy /Y "%QT_DIR%\bin\Qt6QmlWorkerScript.dll"                                                     "%TARGET%\"
xcopy /Y "%QT_DIR%\bin\Qt6Quick.dll"                                                               "%TARGET%\"
xcopy /Y "%QT_DIR%\bin\Qt6QuickControls2.dll"                                                      "%TARGET%\"
xcopy /Y "%QT_DIR%\bin\Qt6QuickControls2Impl.dll"                                                  "%TARGET%\"
xcopy /Y "%QT_DIR%\bin\Qt6QuickDialogs2.dll"                                                       "%TARGET%\"
xcopy /Y "%QT_DIR%\bin\Qt6QuickDialogs2QuickImpl.dll"                                              "%TARGET%\"
xcopy /Y "%QT_DIR%\bin\Qt6QuickDialogs2Utils.dll"                                                  "%TARGET%\"
xcopy /Y "%QT_DIR%\bin\Qt6QuickTemplates2.dll"                                                     "%TARGET%\"
xcopy /Y "%QT_DIR%\bin\Qt6Svg.dll"                                                                 "%TARGET%\"
:: Copy QtQuick
xcopy /Y "%QT_DIR%\qml\Qt\labs\settings\qmldir"                                                    "%TARGET%\Qt\labs\settings\"
xcopy /Y "%QT_DIR%\qml\Qt\labs\settings\qmlsettingsplugin.dll"                                     "%TARGET%\Qt\labs\settings\"
xcopy /Y "%QT_DIR%\qml\QtQml\qmldir"                                                               "%TARGET%\QtQml\"
xcopy /Y "%QT_DIR%\qml\QtQml\qmlplugin.dll"                                                        "%TARGET%\QtQml\"
xcopy /Y "%QT_DIR%\qml\QtQml\WorkerScript\qmldir"                                                  "%TARGET%\QtQml\WorkerScript\"
xcopy /Y "%QT_DIR%\qml\QtQml\WorkerScript\workerscriptplugin.dll"                                  "%TARGET%\QtQml\WorkerScript\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Controls\Basic\*.qml"                                               "%TARGET%\QtQuick\Controls\Basic\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Controls\Basic\qmldir"                                              "%TARGET%\QtQuick\Controls\Basic\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Controls\Basic\qtquickcontrols2basicstyleplugin.dll"                "%TARGET%\QtQuick\Controls\Basic\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Controls\Basic\impl\qmldir"                                         "%TARGET%\QtQuick\Controls\Basic\impl\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Controls\Basic\impl\qtquickcontrols2basicstyleimplplugin.dll"       "%TARGET%\QtQuick\Controls\Basic\impl\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Controls\impl\qmldir"                                               "%TARGET%\QtQuick\Controls\impl\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Controls\impl\qtquickcontrols2implplugin.dll"                       "%TARGET%\QtQuick\Controls\impl\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Controls\Material\*.qml"                                            "%TARGET%\QtQuick\Controls\Material\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Controls\Material\qmldir"                                           "%TARGET%\QtQuick\Controls\Material\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Controls\Material\qtquickcontrols2materialstyleplugin.dll"          "%TARGET%\QtQuick\Controls\Material\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Controls\Material\impl\*.qml"                                       "%TARGET%\QtQuick\Controls\Material\impl\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Controls\Material\impl\qmldir"                                      "%TARGET%\QtQuick\Controls\Material\impl\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Controls\Material\impl\qtquickcontrols2materialstyleimplplugin.dll" "%TARGET%\QtQuick\Controls\Material\impl\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Controls\qmldir"                                                    "%TARGET%\QtQuick\Controls\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Controls\qtquickcontrols2plugin.dll"                                "%TARGET%\QtQuick\Controls\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Dialogs\qmldir"                                                     "%TARGET%\QtQuick\Dialogs\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Dialogs\qtquickdialogsplugin.dll"                                   "%TARGET%\QtQuick\Dialogs\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Dialogs\quickimpl\qml\+Material\*.qml"                              "%TARGET%\QtQuick\Dialogs\quickimpl\qml\+Material\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Dialogs\quickimpl\qml\*.qml"                                        "%TARGET%\QtQuick\Dialogs\quickimpl\qml\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Dialogs\quickimpl\qmldir"                                           "%TARGET%\QtQuick\Dialogs\quickimpl\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Dialogs\quickimpl\qtquickdialogs2quickimplplugin.dll"               "%TARGET%\QtQuick\Dialogs\quickimpl\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Layouts\qmldir"                                                     "%TARGET%\QtQuick\Layouts\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Layouts\qquicklayoutsplugin.dll"                                    "%TARGET%\QtQuick\Layouts\"
xcopy /Y "%QT_DIR%\qml\QtQuick\qmldir"                                                             "%TARGET%\QtQuick\"
xcopy /Y "%QT_DIR%\qml\QtQuick\qtquick2plugin.dll"                                                 "%TARGET%\QtQuick\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Templates\qmldir"                                                   "%TARGET%\QtQuick\Templates\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Templates\qtquicktemplates2plugin.dll"                              "%TARGET%\QtQuick\Templates\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Window\qmldir"                                                      "%TARGET%\QtQuick\Window\"
xcopy /Y "%QT_DIR%\qml\QtQuick\Window\quickwindowplugin.dll"                                       "%TARGET%\QtQuick\Window\"

:: Copy ffmpeg
:: xcopy /Y "%FFMPEG_DIR%\bin\x64\avcodec-58.dll"    "%TARGET%\"
:: xcopy /Y "%FFMPEG_DIR%\bin\x64\avfilter-7.dll"    "%TARGET%\"
:: xcopy /Y "%FFMPEG_DIR%\bin\x64\avformat-58.dll"   "%TARGET%\"
:: xcopy /Y "%FFMPEG_DIR%\bin\x64\avutil-56.dll"     "%TARGET%\"
:: xcopy /Y "%FFMPEG_DIR%\bin\x64\swresample-3.dll"  "%TARGET%\"
:: xcopy /Y "%FFMPEG_DIR%\bin\x64\swscale-5.dll"     "%TARGET%\"
:: xcopy /Y "%FFMPEG_DIR%\bin\x64\postproc-55.dll"   "%TARGET%\"
xcopy /Y "%FFMPEG_DIR%\bin\avcodec-59.dll"    "%TARGET%\"
xcopy /Y "%FFMPEG_DIR%\bin\avdevice-59.dll"   "%TARGET%\"
xcopy /Y "%FFMPEG_DIR%\bin\avfilter-8.dll"    "%TARGET%\"
xcopy /Y "%FFMPEG_DIR%\bin\avformat-59.dll"   "%TARGET%\"
xcopy /Y "%FFMPEG_DIR%\bin\avutil-57.dll"     "%TARGET%\"
xcopy /Y "%FFMPEG_DIR%\bin\swresample-4.dll"  "%TARGET%\"
xcopy /Y "%FFMPEG_DIR%\bin\swscale-6.dll"     "%TARGET%\"
:: xcopy /Y "%FFMPEG_DIR%\bin\postproc-55.dll"   "%TARGET%\"

:: Copy OpenCV
xcopy /Y "%OPENCV_DIR%\opencv_calib*.dll"      "%TARGET%\"
xcopy /Y "%OPENCV_DIR%\opencv_cor*.dll"        "%TARGET%\"
xcopy /Y "%OPENCV_DIR%\opencv_features*.dll"   "%TARGET%\"
xcopy /Y "%OPENCV_DIR%\opencv_flan*.dll"       "%TARGET%\"
xcopy /Y "%OPENCV_DIR%\opencv_imgpro*.dll"     "%TARGET%\"
xcopy /Y "%OPENCV_DIR%\opencv_vide*.dll"       "%TARGET%\"
xcopy /Y "%OPENCV_DIR%\zlib*.dll"              "%TARGET%\"
del "%TARGET%\opencv_*videoio*"

:: Copy Gyroflow
xcopy /Y "%CARGO_TARGET%\mdk.dll"                                      "%TARGET%\"
echo F | xcopy /Y "%CARGO_TARGET%\gyroflow.exe"                        "%TARGET%\Gyroflow.exe"
xcopy /Y "%PROJECT_DIR%\_deployment\windows\Gyroflow_with_console.bat" "%TARGET%\"
xcopy /Y /E "%PROJECT_DIR%\resources\camera_presets\*"                 "%TARGET%\camera_presets\"

:: Other
xcopy /Y "%QT_DIR%\bin\d3dcompiler*.dll"                             "%TARGET%\"
:: vc_redist.x64.exe