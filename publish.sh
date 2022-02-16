#/bin/bash
read -p "Enter new version: " ver
sed -i'' -E "0,/version = \"[0-9\.a-z-]+\"/s//version = \"${ver}\"/" Cargo.toml
sed -i'' -E "0,/version = \"[0-9\.a-z-]+\"/s//version = \"${ver}\"/" src/core/Cargo.toml
sed -i'' -E "0,/APP_VERSION=[0-9\.a-z-]+/s//APP_VERSION=${ver}/" _deployment/deploy-linux.sh
sed -i'' -E "0,/Gyroflow v[0-9\.a-z-]+/s//Gyroflow v${ver}/" _deployment/deploy-macos.sh
sed -i'' -E "0,/APP_VERSION=[0-9\.a-z-]+/s//APP_VERSION=${ver}/" _deployment/linux/build-appimage.sh
sed -i'' -E "0,/versionName=\"[0-9\.a-z-]+\"/s//versionName=\"${ver}\"/" _deployment/android/AndroidManifest.xml
git commit -a -m "Release v${ver}"
git tag -a "v${ver}" -m "Release v${ver}"
git.exe push origin
git.exe push origin "v${ver}"