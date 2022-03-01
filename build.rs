// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

fn main() {
    let qt_include_path = std::env::var("DEP_QT_INCLUDE_PATH").unwrap();
    let qt_library_path = std::env::var("DEP_QT_LIBRARY_PATH").unwrap();
    let qt_version      = std::env::var("DEP_QT_VERSION").unwrap();

    let mut config = cpp_build::Config::new();

    for f in std::env::var("DEP_QT_COMPILE_FLAGS").unwrap().split_terminator(";") {
        config.flag(f);
    }
    
    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=z");
        println!("cargo:rustc-link-lib=bz2");
        println!("cargo:rustc-link-lib=xml2");
        println!("cargo:rustc-link-lib=framework=AudioToolbox");
        println!("cargo:rustc-link-lib=framework=VideoToolbox");
        println!("cargo:rustc-link-lib=framework=QuartzCore");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=CoreMedia");
        println!("cargo:rustc-link-lib=framework=CoreAudio");
        println!("cargo:rustc-link-lib=framework=CoreVideo");
        println!("cargo:rustc-link-lib=framework=CoreServices");
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rustc-link-lib=framework=OpenGL");
    }

    let mut public_include = |name| {
        if cfg!(target_os = "macos") {
            config.include(format!("{}/{}.framework/Headers/", qt_library_path, name));
        }
        config.include(format!("{}/{}", qt_include_path, name));
    };
    public_include("QtCore");
    public_include("QtGui");
    public_include("QtQuick");
    public_include("QtQml");
    public_include("QtQuickControls2");

    let mut private_include = |name| {
        if cfg!(target_os = "macos") {
            config.include(format!("{}/{}.framework/Headers/{}",       qt_library_path, name, qt_version));
            config.include(format!("{}/{}.framework/Headers/{}/{}",    qt_library_path, name, qt_version, name));
        }
        config.include(format!("{}/{}/{}",    qt_include_path, name, qt_version))
              .include(format!("{}/{}/{}/{}", qt_include_path, name, qt_version, name));
    };
    private_include("QtCore");
    private_include("QtGui");
    private_include("QtQuick");
    private_include("QtQml");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    match target_os.as_str() {
        "android" => {
            println!("cargo:rustc-link-search={}/lib/arm64-v8a", std::env::var("FFMPEG_DIR").unwrap());
            println!("cargo:rustc-link-search={}/lib", std::env::var("FFMPEG_DIR").unwrap());
            println!("cargo:rustc-link-search={}/installed/arm64-android/lib", std::env::var("VCPKG_ROOT").unwrap());
            config.include(format!("{}/installed/arm64-android/include", std::env::var("VCPKG_ROOT").unwrap()));
            config.include(format!("{}/include", std::env::var("FFMPEG_DIR").unwrap()));
        },
        "macos" => {
            println!("cargo:rustc-link-search={}/lib", std::env::var("FFMPEG_DIR").unwrap());
            println!("cargo:rustc-link-lib=static=x264");
            println!("cargo:rustc-link-lib=static=x265");
        },
        "linux" => {
            println!("cargo:rustc-link-search={}", std::env::var("OPENCV_LINK_PATHS").unwrap());
            println!("cargo:rustc-link-search={}/lib/amd64", std::env::var("FFMPEG_DIR").unwrap());
            println!("cargo:rustc-link-search={}/lib", std::env::var("FFMPEG_DIR").unwrap());
            println!("cargo:rustc-link-lib=static=z");
            // assumes that OpenCV has been downloaded using vcpgk and libraries are in static form (default triplets of vcpgk for linux provide static libs)
            std::env::var("OPENCV_LINK_LIBS").unwrap().split(',').for_each(|lib| println!("cargo:rustc-link-lib=static={}", lib.trim()));
        },
        "windows" => {
            println!("cargo:rustc-link-search={}/lib/x64", std::env::var("FFMPEG_DIR").unwrap());
            println!("cargo:rustc-link-search={}/lib", std::env::var("FFMPEG_DIR").unwrap());
            let mut res = winres::WindowsResource::new();
            res.set_icon("resources/app_icon.ico");
            res.set("FileVersion", env!("CARGO_PKG_VERSION"));
            res.set("ProductVersion", env!("CARGO_PKG_VERSION"));
            res.set("ProductName", "Gyroflow");
            res.set("FileDescription", &format!("Gyroflow v{}", env!("CARGO_PKG_VERSION")));
            res.compile().unwrap();
        }
        tos => panic!("unknown target os {:?}!", tos)
    }

    if let Ok(time) = std::time::SystemTime::now().duration_since(std::time::SystemTime::UNIX_EPOCH) {
        println!("cargo:rustc-env=BUILD_TIME={}", (time.as_secs() - 1642516578) / 600); // New version every 10 minutes
    }

    config
        .include(&qt_include_path)
        .build("src/gyroflow.rs");

}
