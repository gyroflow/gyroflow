set windows-shell := ["powershell.exe", "-NoLogo", "-Command"]

BaseFile := os()

run *param:
    just -f _scripts/{{BaseFile}}.just run {{param}}

test *param:
    just -f _scripts/{{BaseFile}}.just test {{param}}

build *param:
    just -f _scripts/{{BaseFile}}.just build {{param}}

build-debug *param:
    just -f _scripts/{{BaseFile}}.just build-debug {{param}}

debug *param:
    just -f _scripts/{{BaseFile}}.just debug {{param}}

profile *param:
    just -f _scripts/{{BaseFile}}.just profile {{param}}

clippy *param:
    just -f _scripts/{{BaseFile}}.just clippy {{param}}

install-deps *param:
    just -f _scripts/{{BaseFile}}.just install-deps {{param}}

deploy *param:
    just -f _scripts/{{BaseFile}}.just deploy {{param}}

bundle *param:
    just -f _scripts/{{BaseFile}}.just bundle {{param}}

android *param:
    just -f _scripts/android.just {{param}}

ios *param:
    just -f _scripts/ios.just {{param}}

publish version:
    #!/bin/bash
    git clone --depth 1 git@github.com:gyroflow/gyroflow.git __publish
    pushd __publish
    sed -i'' -E "0,/ProjectVersion := \"[0-9\.a-z-]+\"/s//ProjectVersion := \"{{version}}\"/" _scripts/common.just
    sed -i'' -E "0,/version = \"[0-9\.a-z-]+\"/s//version = \"{{version}}\"/" Cargo.toml
    sed -i'' -E "0,/version = \"[0-9\.a-z-]+\"/s//version = \"{{version}}\"/" src/core/Cargo.toml
    sed -i'' -E "/<key>CFBundleShortVersionString<.key>/ s/<string>[0-9\.a-z-]+<.string>/<string>{{version}}<\/string>/" _deployment/mac/Gyroflow.app/Contents/Info.plist
    sed -i'' -E "/<key>CFBundleVersion<.key>/ s/<string>[0-9\.a-z-]+<.string>/<string>{{version}}<\/string>/" _deployment/mac/Gyroflow.app/Contents/Info.plist
    sed -i'' -E "0,/Gyroflow v[0-9\.a-z-]+/s//Gyroflow v{{version}}/" src/cli.rs
    sed -i'' -E "0,/versionName=\"[0-9\.a-z-]+\"/s//versionName=\"{{version}}\"/" _deployment/android/AndroidManifest.xml
    sed -i'' -E "/<key>CFBundleShortVersionString<.key>/ s/<string>[0-9\.a-z-]+<.string>/<string>{{version}}<\/string>/" _deployment/ios/Info.plist
    sed -i'' -E "/<key>CFBundleVersion<.key>/ s/<string>[0-9\.a-z-]+<.string>/<string>{{version}}<\/string>/" _deployment/ios/Info.plist
    sed -i'' -E "0,/Identity Version=\"[0-9\.a-z-]+\"/s//Identity Version=\"{{version}}.0\"/" _deployment/windows/AppxManifest.xml
    git commit -a -m "Release v{{version}}"
    git tag -a "v{{version}}" -m "Release v{{version}}"
    git push origin
    git push origin "v{{version}}"
    popd
    rm -rf __publish
    git pull
