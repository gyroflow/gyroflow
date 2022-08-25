#/bin/bash

APP_PKG=$PWD/Gyroflow
export APP_DIR=$PWD/AppDir
export APP_VERSION=1.2.0

rm -rf $APP_DIR
mkdir -p $APP_DIR/usr/share/icons
cp -f gyroflow.png $APP_DIR/usr/share/icons/
cp -f gyroflow.svg $APP_DIR/usr/share/icons/

cp -rf $APP_PKG/* $APP_DIR/
appimage-builder --recipe $PWD/AppImageBuilder.yml
chmod +x Gyroflow-$APP_VERSION-x86_64.AppImage
./Gyroflow-$APP_VERSION-x86_64.AppImage