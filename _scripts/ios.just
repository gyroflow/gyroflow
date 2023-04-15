!include common.just

QtVersionIOS := "6.5.0"

IPHONESDK := `xcode-select -p` + "/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk"
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_apple_ios := "--target=arm64-apple-ios -arch arm64 -miphoneos-version-min=15 -isysroot " + IPHONESDK
export CFLAGS_aarch64_apple_darwin := "-mmacosx-version-min=10.14"
export IPHONEOS_DEPLOYMENT_TARGET := "15.0"

# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ Dependencies ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

[no-cd]
install-deps:
    #!/bin/bash
    brew install p7zip pkg-config ios-deploy
    rustup target add aarch64-apple-ios
    xcode-select --install
    export DYLD_FALLBACK_LIBRARY_PATH=""
    export MACOSX_DEPLOYMENT_TARGET=""

    cd {{ExtDir}}

    # Install vcpkg
    git clone --depth 1 https://github.com/Microsoft/vcpkg.git
    ./vcpkg/bootstrap-vcpkg.sh -disableMetrics

    if [ ! -d "{{ExtDir}}/{{QtVersionIOS}}/ios" ]; then
        # Install Qt
        pip3 install -U pip aqtinstall
        python3 -m aqt install-qt mac ios {{QtVersionIOS}}
    fi

    if [ ! -f "{{ExtDir}}/vcpkg/installed/arm64-ios/lib/libopencv_core4.a" ]; then
        echo "Installing OpenCV from vcpkg"
        # Install OpenCV
        ./vcpkg/vcpkg install "opencv4[core]:arm64-ios"
    fi

    if [ ! -d "ffmpeg-master-iOS-gpl-lite" ]; then
        echo "Downloading ffmpeg for iOS"

        # Download and extract ffmpeg
        curl -L https://sourceforge.net/projects/avbuild/files/iOS/ffmpeg-master-iOS-gpl-lite.tar.xz/download -o ffmpeg.tar.xz
        7z x -aoa ffmpeg.tar.xz
        tar -xf ffmpeg.tar
        rm ffmpeg.tar ffmpeg.tar.xz
    fi

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
    #!/bin/bash
    : "${PROJECT_DIR:=$(cd "{{ProjectDir}}"; pwd -P)}"

    security -v unlock-keychain -p 1234 login.keychain

    APP_BUNDLE=$PROJECT_DIR/_deployment/_binaries/ios/gyroflow.app

    rm -rf $APP_BUNDLE
    rm -f $PROJECT_DIR/target/aarch64-apple-ios/release/gyroflow

    mkdir -p $APP_BUNDLE

    OPENCV_DIR="$PROJECT_DIR/ext/vcpkg/installed/arm64-ios"
    export FFMPEG_DIR="$PROJECT_DIR/ext/ffmpeg-master-iOS-gpl-lite"
    export PATH="$PROJECT_DIR/ext/{{QtVersionIOS}}/ios/bin:$PATH"

    export OPENCV_LINK_LIBS="opencv_core4,opencv_calib3d4,opencv_features2d4,opencv_imgproc4,opencv_video4,opencv_flann4"
    export OPENCV_LINK_PATHS=$OPENCV_DIR/lib
    export OPENCV_INCLUDE_PATHS=$OPENCV_DIR/include/

    cargo build --target aarch64-apple-ios --profile release

    cp -f $PROJECT_DIR/target/aarch64-apple-ios/release/gyroflow $APP_BUNDLE/gyroflow
    cp -rf $PROJECT_DIR/target/aarch64-apple-ios/Frameworks $APP_BUNDLE/Frameworks
    cp -rf $PROJECT_DIR/_deployment/ios/Info.plist $APP_BUNDLE/Info.plist
    cp -rf $PROJECT_DIR/_deployment/ios/PkgInfo $APP_BUNDLE/PkgInfo
    cp -rf $PROJECT_DIR/_deployment/ios/Entitlements.plist $APP_BUNDLE/../entitlements.xcent

    xcrun actool $PROJECT_DIR/_deployment/ios/Resources/Images.xcassets --compile $APP_BUNDLE --platform iphoneos --minimum-deployment-target 15 --app-icon AppIcon --output-partial-info-plist $APP_BUNDLE/test.plist

    #cp -f $PROJECT_DIR/_deployment/ios/Gyroflow_dev.mobileprovision $APP_BUNDLE/embedded.mobileprovision

    /Applications/Xcode.app/Contents/Developer/usr/bin/ibtool --errors --warnings --notices --module gyroflow --output-partial-info-plist $APP_BUNDLE/../LaunchScreen-SBPartialInfo.plist --auto-activate-custom-fonts --target-device iphone --target-device ipad --minimum-deployment-target 14.0 --output-format human-readable-text --compilation-directory $APP_BUNDLE/../ $PROJECT_DIR/_deployment/ios/LaunchScreen.storyboard
    /Applications/Xcode.app/Contents/Developer/usr/bin/ibtool --errors --warnings --notices --module gyroflow --target-device iphone --target-device ipad --minimum-deployment-target 14.0 --output-format human-readable-text --link $APP_BUNDLE $APP_BUNDLE/../LaunchScreen.storyboardc

    #/usr/bin/derq query -f xml -i $PROJECT_DIR/_deployment/ios/Entitlements.plist -o $APP_BUNDLE/../gyroflow.app.xcent.der --raw

    #/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/dsymutil $APP_BUNDLE/gyroflow -o $APP_BUNDLE/../gyroflow.app.dSYM

    IDENTITY="612D010FA5522A25CE1F1D5A38B84DCD2798704B"
    codesign -vvvv --force --timestamp=none --generate-entitlement-der --sign $IDENTITY ${APP_BUNDLE}/Frameworks/mdk.framework/mdk
    for lib in ${APP_BUNDLE}/*; do
        codesign -vvvv --force --timestamp=none --generate-entitlement-der --sign $IDENTITY ${lib}
    done

    codesign -vvvv --force --sign $IDENTITY --entitlements $APP_BUNDLE/../entitlements.xcent --timestamp=none --generate-entitlement-der $APP_BUNDLE

    codesign -vvvv --deep --verify $APP_BUNDLE

    touch -c $APP_BUNDLE

    if [ -f "$APP_BUNDLE/gyroflow" ]; then
        ios-deploy --bundle $APP_BUNDLE --debug
    fi
