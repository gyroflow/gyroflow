!include common.just

# Important! For development/debugging the provisioning profile needs to be "Development"
# For distribution - it needs to be a distribution provisioning profile.

# Make sure to unlock the keychain if using ssh: security -v unlock-keychain -p 1234 login.keychain
# To list available certificates: security find-identity -v -p codesigning

TeamID                        := "RR3ZC2L4DF"
BundleIdentifier              := "xyz.gyroflow"
DevelopmentCertificateSHA     := "612D010FA5522A25CE1F1D5A38B84DCD2798704B"
DevelopmentProvisioning       := ProjectDir / "_deployment/ios/Gyroflow_dev.mobileprovision"
DevelopmentProvisioningName   := "Gyroflow dev"
DistributionCertificateSHA    := "7F82D6013B8F6D45F88FE547438744BD64750C5B"
DistributionProvisioning      := ProjectDir / "_deployment/ios/Gyroflow_dist.mobileprovision"
DistributionProvisioningName  := "Gyroflow dist"

AppBundle := ProjectDir / "_deployment/_binaries/ios/Payload/Gyroflow.app"

QtVersionIOS := "6.6.1"

IPHONESDK := `xcode-select -p` + "/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk"
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_apple_ios := "--target=arm64-apple-ios -arch arm64 -miphoneos-version-min=14 -isysroot " + IPHONESDK
export CFLAGS_aarch64_apple_darwin := "-mmacosx-version-min=10.14"
export IPHONEOS_DEPLOYMENT_TARGET := "14.0"

# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ Dependencies ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

[no-cd]
install-deps:
    #!/bin/bash
    set -e
    brew install p7zip python pkg-config ios-deploy
    rustup target add aarch64-apple-ios
    xcode-select --install || true
    export DYLD_FALLBACK_LIBRARY_PATH=""
    #export MACOSX_DEPLOYMENT_TARGET=""

    mkdir -p {{ExtDir}}
    cd {{ExtDir}}

    # Install vcpkg
    git clone --depth 1 https://github.com/Microsoft/vcpkg.git || true
    ./vcpkg/bootstrap-vcpkg.sh -disableMetrics

    if [ ! -d "{{ExtDir}}/{{QtVersionIOS}}/ios" ] || [ ! -d "{{ExtDir}}/{{QtVersionIOS}}/macos" ]; then
        # Install Qt
        pip3 install -U pip aqtinstall
        python3 -m aqt install-qt mac desktop {{QtVersionIOS}}
        python3 -m aqt install-qt mac ios {{QtVersionIOS}}

        # Replace the Qt Quick Dialogs file with a patch to fix the file selection bug
        rm -f {{ExtDir}}/{{QtVersionIOS}}/ios/lib/libQt6QuickDialogs2.a
        curl -L https://api.gyroflow.xyz/sdk/Qt-ios-{{QtVersionIOS}}-libQt6QuickDialogs2.a -o {{ExtDir}}/{{QtVersionIOS}}/ios/lib/libQt6QuickDialogs2.a
    fi

    if [ ! -f "{{ExtDir}}/vcpkg/installed/arm64-ios/lib/libopencv_core4.a" ]; then
        echo "Installing OpenCV from vcpkg"
        # Install OpenCV
        ./vcpkg/vcpkg install "opencv4[core]:arm64-ios"
        rm -rf ./vcpkg/buildtrees
        rm -rf ./vcpkg/downloads
    fi

    if [ ! -d "ffmpeg-6.1-iOS-gpl-lite" ]; then
        echo "Downloading ffmpeg for iOS"

        # Download and extract ffmpeg
        curl -L https://sourceforge.net/projects/avbuild/files/iOS/ffmpeg-6.1-iOS-gpl-lite.tar.xz/download -o ffmpeg.tar.xz
        7z x -aoa ffmpeg.tar.xz
        tar -xf ffmpeg.tar
        rm ffmpeg.tar ffmpeg.tar.xz
    fi

# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ Development ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

[no-cd]
run:
    #!/bin/bash
    just --unstable -f {{justfile()}} build-and-package release development

    ios-deploy --bundle {{AppBundle}} -d -W

[no-cd]
debug:
    #!/bin/bash
    just --unstable -f {{justfile()}} build-and-package debug development

    ios-deploy --bundle {{AppBundle}} --debug

# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ Deployment ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
# ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

[no-cd]
build-and-package profile is_dist:
    #!/bin/bash
    set -e

    rm -rf {{ProjectDir}}/_deployment/_binaries/ios/
    rm -f {{ProjectDir}}/target/aarch64-apple-ios/{{profile}}/gyroflow

    mkdir -p {{AppBundle}}

    export FFMPEG_DIR="{{ProjectDir}}/ext/ffmpeg-6.1-iOS-gpl-lite"
    export PATH="{{ProjectDir}}/ext/{{QtVersionIOS}}/ios/bin:/usr/libexec/:$PATH"

    export OPENCV_LINK_LIBS="opencv_core4,opencv_calib3d4,opencv_features2d4,opencv_imgproc4,opencv_video4,opencv_flann4,opencv_stitching4"
    export OPENCV_LINK_PATHS={{ProjectDir}}/ext/vcpkg/installed/arm64-ios/lib
    export OPENCV_INCLUDE_PATHS={{ProjectDir}}/ext/vcpkg/installed/arm64-ios/include/

    if [ "{{profile}}" == "debug" ]; then
        cargo build --target aarch64-apple-ios
    else
        cargo build --target aarch64-apple-ios --profile {{profile}}
    fi

    if [ -z "$SIGN_KEY" ] && [ -z "$PROVISIONING_PROFILE" ]; then
        if [ "{{is_dist}}" == "distribution" ]; then
            SIGN_KEY="{{DistributionCertificateSHA}}"
            PROVISIONING_PROFILE="{{DistributionProvisioning}}"
        else
            SIGN_KEY="{{DevelopmentCertificateSHA}}"
            PROVISIONING_PROFILE="{{DevelopmentProvisioning}}"
        fi
    fi

    if [ -f "{{ProjectDir}}/target/aarch64-apple-ios/{{profile}}/gyroflow" ]; then
        cp -f {{ProjectDir}}/target/aarch64-apple-ios/{{profile}}/gyroflow {{AppBundle}}/gyroflow
        cp -rf {{ProjectDir}}/target/aarch64-apple-ios/Frameworks {{AppBundle}}/Frameworks
        cp -rf {{ProjectDir}}/_deployment/ios/Info.plist {{AppBundle}}/Info.plist
        cp -rf {{ProjectDir}}/_deployment/ios/PkgInfo {{AppBundle}}/PkgInfo
        PlistBuddy -x -c "Set :CFBundleIdentifier {{BundleIdentifier}}" {{AppBundle}}/Info.plist
        PlistBuddy -x -c "Set :CFBundleShortVersionString {{ProjectVersion}}" {{AppBundle}}/Info.plist
        PlistBuddy -x -c "Set :CFBundleVersion {{ProjectVersion}}" {{AppBundle}}/Info.plist
        if ! [ -z "$GITHUB_RUN_NUMBER" ]; then
            PlistBuddy -x -c "Set :CFBundleVersion \"{{ProjectVersion}}.$GITHUB_RUN_NUMBER\"" {{AppBundle}}/Info.plist
        fi
        PATCH_NUM="0"
        if [ "$IS_MANUAL_DISPATCH" == "true" ]; then
            PATCH_NUM="1"
        fi
        PlistBuddy -x -c "Set :CFBundleShortVersionString \"`echo "{{ProjectVersion}}" | sed -E 's/[^0-9\.]//' | sed -E 's/\.([0-9]+)$/\1/'`.$PATCH_NUM\"" {{AppBundle}}/Info.plist

        ICON={{ProjectDir}}/resources/icon_1080.png
        ICON_OUT={{ProjectDir}}/_deployment/ios/Resources/Images.xcassets/AppIcon.appiconset
        sips -z 40 40     $ICON --out $ICON_OUT/AppIcon-20@2x.png
        sips -z 40 40     $ICON --out $ICON_OUT/AppIcon-20@2x~ipad.png
        sips -z 60 60     $ICON --out $ICON_OUT/AppIcon-20@3x.png
        sips -z 20 20     $ICON --out $ICON_OUT/AppIcon-20~ipad.png
        sips -z 29 29     $ICON --out $ICON_OUT/AppIcon-29.png
        sips -z 58 58     $ICON --out $ICON_OUT/AppIcon-29@2x.png
        sips -z 58 58     $ICON --out $ICON_OUT/AppIcon-29@2x~ipad.png
        sips -z 87 87     $ICON --out $ICON_OUT/AppIcon-29@3x.png
        sips -z 29 29     $ICON --out $ICON_OUT/AppIcon-29~ipad.png
        sips -z 80 80     $ICON --out $ICON_OUT/AppIcon-40@2x.png
        sips -z 80 80     $ICON --out $ICON_OUT/AppIcon-40@2x~ipad.png
        sips -z 120 120   $ICON --out $ICON_OUT/AppIcon-40@3x.png
        sips -z 40 40     $ICON --out $ICON_OUT/AppIcon-40~ipad.png
        sips -z 120 120   $ICON --out $ICON_OUT/AppIcon-60@2x~car.png
        sips -z 180 180   $ICON --out $ICON_OUT/AppIcon-60@3x~car.png
        sips -z 167 167   $ICON --out $ICON_OUT/AppIcon-83.5@2x~ipad.png
        sips -z 120 120   $ICON --out $ICON_OUT/AppIcon@2x.png
        sips -z 152 152   $ICON --out $ICON_OUT/AppIcon@2x~ipad.png
        sips -z 180 180   $ICON --out $ICON_OUT/AppIcon@3x.png
        sips -z 1024 1024 $ICON --out $ICON_OUT/AppIcon~ios-marketing.png
        sips -z 76 76     $ICON --out $ICON_OUT/AppIcon~ipad.png

        DEFAULT_iOS_SDK_VERSION=`xcrun --sdk iphoneos --show-sdk-version`
        PlistBuddy -x -c "Add :BuildMachineOSBuild string `sw_vers -buildVersion`" {{AppBundle}}/Info.plist
        PlistBuddy -x -c "Add :DTCompiler string `defaults read $(xcode-select -p)/../info DTCompiler`" {{AppBundle}}/Info.plist
        PlistBuddy -x -c "Add :DTPlatformBuild string `xcrun --sdk iphoneos --show-sdk-build-version`" {{AppBundle}}/Info.plist
        PlistBuddy -x -c "Add :DTPlatformName string iphoneos" {{AppBundle}}/Info.plist
        PlistBuddy -x -c "Add :DTPlatformVersion string $DEFAULT_iOS_SDK_VERSION" {{AppBundle}}/Info.plist
        PlistBuddy -x -c "Add :DTSDKBuild string `xcrun --sdk iphoneos --show-sdk-build-version`" {{AppBundle}}/Info.plist
        PlistBuddy -x -c "Add :DTSDKName string iphoneos$DEFAULT_iOS_SDK_VERSION" {{AppBundle}}/Info.plist
        PlistBuddy -x -c "Add :DTXcode string `defaults read $(xcode-select -p)/../info DTXcode`" {{AppBundle}}/Info.plist
        PlistBuddy -x -c "Add :DTXcodeBuild string `xcodebuild -version | grep 'Build version' | sed 's/^Build version //'`" {{AppBundle}}/Info.plist

        cat {{AppBundle}}/Info.plist

        # Generate entitlements
        PlistBuddy -x -c "Add :application-identifier string {{TeamID}}.{{BundleIdentifier}}" {{AppBundle}}/../../entitlements.xcent
        PlistBuddy -x -c "Add :com.apple.developer.team-identifier string {{TeamID}}" {{AppBundle}}/../../entitlements.xcent
        PlistBuddy -x -c "Add :com.apple.developer.kernel.increased-memory-limit bool true" {{AppBundle}}/../../entitlements.xcent
        if [ "{{is_dist}}" == "distribution" ]; then
            PlistBuddy -x -c "Add :get-task-allow bool false" {{AppBundle}}/../../entitlements.xcent
        else
            PlistBuddy -x -c "Add :get-task-allow bool true" {{AppBundle}}/../../entitlements.xcent
        fi
        PlistBuddy -x -c "Add :keychain-access-groups array" {{AppBundle}}/../../entitlements.xcent
        PlistBuddy -x -c "Add :keychain-access-groups:0 string {{TeamID}}.{{BundleIdentifier}}" {{AppBundle}}/../../entitlements.xcent

        actool {{ProjectDir}}/_deployment/ios/Resources/Images.xcassets --compile {{AppBundle}} --platform iphoneos --minimum-deployment-target 15 --app-icon AppIcon --output-partial-info-plist {{AppBundle}}/../../AppIcon.plist

        ibtool --errors --warnings --notices --module gyroflow --target-device iphone --target-device ipad --minimum-deployment-target 14.0 --output-format human-readable-text --auto-activate-custom-fonts --compilation-directory {{AppBundle}}/../../ {{ProjectDir}}/_deployment/ios/LaunchScreen.storyboard
        ibtool --errors --warnings --notices --module gyroflow --target-device iphone --target-device ipad --minimum-deployment-target 14.0 --output-format human-readable-text --link {{AppBundle}} {{AppBundle}}/../../LaunchScreen.storyboardc

        cp -f "$PROVISIONING_PROFILE" {{AppBundle}}/embedded.mobileprovision
        #derq query -f xml -i {{ProjectDir}}/_deployment/ios/Entitlements.plist -o {{AppBundle}}/../../Gyroflow.app.xcent.der --raw
        dsymutil {{AppBundle}}/gyroflow -o {{AppBundle}}/../../../Gyroflow.app.dSYM
        mkdir -p {{AppBundle}}/../../Symbols
        xcrun symbols -noTextInSOD -noDaemon -arch all -symbolsPackageDir {{AppBundle}}/../../Symbols {{AppBundle}}/../../../Gyroflow.app.dSYM
        rm -rf {{AppBundle}}/../../../Gyroflow.app.dSYM

        codesign -vvvv --force --generate-entitlement-der --preserve-metadata=identifier,flags,runtime --sign $SIGN_KEY {{AppBundle}}/Frameworks/mdk.framework
        codesign -vvvv --force --generate-entitlement-der --preserve-metadata=identifier,flags,runtime --sign $SIGN_KEY --entitlements {{AppBundle}}/../../entitlements.xcent {{AppBundle}}

        codesign -vvvv --deep --strict --verify {{AppBundle}}
    fi

[no-cd]
deploy *param:
    #!/bin/bash
    set -e
    export PATH="/usr/libexec/:$PATH"

    just --unstable -f {{justfile()}} build-and-package deploy distribution

    pushd {{AppBundle}}/../../
        zip -r ../Gyroflow.ipa Payload Symbols
    popd
    ios-deploy --bundle {{AppBundle}}

    # ---------------------- XCode archive way ----------------------
    # This shouldn't be normally needed, but if anything changes in the build process in the future, it's more "official" way
    if false; then
        if true; then
            SIGN_KEY="{{DistributionCertificateSHA}}"
            PROVISIONING_PROFILE="{{DistributionProvisioningName}}"
        else
            SIGN_KEY="{{DevelopmentCertificateSHA}}"
            PROVISIONING_PROFILE="{{DevelopmentProvisioningName}}"
        fi
        # For app store
        TYPE="app-store"
        # For ad-hoc
        TYPE="developer-id"

        mkdir -p {{ProjectDir}}/_deployment/_binaries/ios/Gyroflow.xcarchive/Products/Applications
        cp -rf {{AppBundle}} {{ProjectDir}}/_deployment/_binaries/ios/Gyroflow.xcarchive/Products/Applications/

        # Build Archive.plist
        ARCHIVE_PLIST={{ProjectDir}}/_deployment/_binaries/ios/Gyroflow.xcarchive/Info.plist
        PlistBuddy -x -c "Add :ApplicationProperties dict" $ARCHIVE_PLIST
        PlistBuddy -x -c "Add :ApplicationProperties:ApplicationPath string Applications/Gyroflow.app" $ARCHIVE_PLIST
        PlistBuddy -x -c "Add :ApplicationProperties:Architectures array" $ARCHIVE_PLIST
        PlistBuddy -x -c "Add :ApplicationProperties:Architectures:0 string arm64" $ARCHIVE_PLIST
        PlistBuddy -x -c "Add :ApplicationProperties:CFBundleIdentifier string {{BundleIdentifier}}" $ARCHIVE_PLIST
        PlistBuddy -x -c "Add :ApplicationProperties:CFBundleShortVersionString string {{ProjectVersion}}" $ARCHIVE_PLIST
        PlistBuddy -x -c "Add :ApplicationProperties:CFBundleVersion string {{ProjectVersion}}" $ARCHIVE_PLIST
        PlistBuddy -x -c "Add :ApplicationProperties:Team string {{TeamID}}" $ARCHIVE_PLIST
        PlistBuddy -x -c "Add :ArchiveVersion integer 2" $ARCHIVE_PLIST
        PlistBuddy -x -c "Add :CreationDate date \"`date +'%c'`\"" $ARCHIVE_PLIST
        PlistBuddy -x -c "Add :Name string Gyroflow" $ARCHIVE_PLIST
        PlistBuddy -x -c "Add :SchemeName string Gyroflow" $ARCHIVE_PLIST

        # Build Export.plist
        PlistBuddy -x -c "Add :method string $TYPE" {{AppBundle}}/../../Export.plist
        PlistBuddy -x -c "Add :teamID string {{TeamID}}" {{AppBundle}}/../../Export.plist
        PlistBuddy -x -c "Add :signingCertificate string $SIGN_KEY" {{AppBundle}}/../../Export.plist
        PlistBuddy -x -c "Add :uploadSymbols bool true" {{AppBundle}}/../../Export.plist
        PlistBuddy -x -c "Add :provisioningProfiles dict" {{AppBundle}}/../../Export.plist
        PlistBuddy -x -c "Add :provisioningProfiles:{{BundleIdentifier}} string $PROVISIONING_PROFILE" {{AppBundle}}/../../Export.plist

        xcodebuild -sdk iphoneos -exportArchive -archivePath {{ProjectDir}}/_deployment/_binaries/ios/Gyroflow.xcarchive -exportPath {{AppBundle}}/../out/Payload -exportOptionsPlist {{AppBundle}}/../../Export.plist

        pushd {{AppBundle}}/../out/
            rm -f ../Gyroflow.ipa
            rm -f Payload/*.plist
            rm -f Payload/*.log
            zip -r ../Gyroflow.ipa Payload Symbols
        popd
        ios-deploy --bundle {{AppBundle}}/../out/Payload/Gyroflow.app
    fi
    # ---------------------- XCode archive way ----------------------

[no-cd]
build-ipa:
    #!/bin/bash
    set -e
    export PATH="/usr/libexec/:$PATH"

    just --unstable -f {{justfile()}} build-and-package deploy distribution

    pushd {{AppBundle}}/../../
        zip -r ../Gyroflow.ipa Payload Symbols
    popd
