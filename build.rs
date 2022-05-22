// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::process::Command;
use std::path::Path;
use std::env;
use walkdir::WalkDir;

fn compile_qml(dir: &str, qt_include_path: &str, qt_library_path: &str) {
    let mut config = cc::Build::new();
    config.include(&qt_include_path);
    config.include(&format!("{}/QtCore", qt_include_path));
    if cfg!(target_os = "macos") {
        config.include(format!("{}/QtCore.framework/Headers/", qt_library_path));
    }
    for f in std::env::var("DEP_QT_COMPILE_FLAGS").unwrap().split_terminator(';') {
        config.flag(f);
    }

    println!("cargo:rerun-if-changed={}", dir);

    let out_dir = env::var("OUT_DIR").unwrap();
    let out_dir = Path::new(&out_dir);
    let main_dir = env::var("CARGO_MANIFEST_DIR").unwrap();

    let mut files = Vec::new();
    let mut qrc = "<RCC>\n<qresource prefix=\"/\">\n".to_string();
    WalkDir::new(dir).into_iter().flatten().for_each(|entry| {
        let f_name = entry.path().to_string_lossy().replace('\\', "/");
        if f_name.ends_with(".qml") || f_name.ends_with(".js") {
            qrc.push_str(&format!("<file>{}</file>\n", f_name));

            let cpp_name = f_name.replace('/', "_").replace(".qml", ".cpp").replace(".js", ".cpp");
            let cpp_path = out_dir.join(cpp_name).to_string_lossy().to_string();

            config.file(&cpp_path); 
            files.push((f_name, cpp_path));
        }
    });

    let qt_path = std::path::Path::new(qt_library_path).parent().unwrap();
    let compiler_path = if qt_path.join("libexec/qmlcachegen").exists() {
        qt_path.join("libexec/qmlcachegen").to_string_lossy().to_string()
    } else {
        "qmlcachegen".to_string()
    };
    
    qrc.push_str("</qresource>\n</RCC>");
    let qrc_path = Path::new(&main_dir).join("ui.qrc").to_string_lossy().to_string();
    std::fs::write(&qrc_path, qrc).unwrap();

    for (qml, cpp) in &files {
        assert!(Command::new(&compiler_path).args(&["--resource", &qrc_path, "-o", cpp, qml]).status().unwrap().success()); 
    }

    let loader_path = out_dir.join("qmlcache_loader.cpp").to_str().unwrap().to_string();
    assert!(Command::new(&compiler_path).args(&["--resource-file-mapping", &qrc_path, "-o", &loader_path, "ui.qrc"]).status().unwrap().success());

    config.file(&loader_path);

    std::fs::remove_file(&qrc_path).unwrap();

    config.cargo_metadata(false).compile("qmlcache");
    println!("cargo:rustc-link-lib=static:+whole-archive=qmlcache");
}

fn main() {
    let qt_include_path = env::var("DEP_QT_INCLUDE_PATH").unwrap();
    let qt_library_path = env::var("DEP_QT_LIBRARY_PATH").unwrap();
    let qt_version      = env::var("DEP_QT_VERSION").unwrap();

    if let Ok(out_dir) = env::var("OUT_DIR") {
        if out_dir.contains("\\deploy\\build\\") || out_dir.contains("/deploy/build/") {
            compile_qml("src/ui/", &qt_include_path, &qt_library_path);
            println!("cargo:rustc-cfg=compiled_qml");
        }
    }

    let mut config = cpp_build::Config::new();

    for f in env::var("DEP_QT_COMPILE_FLAGS").unwrap().split_terminator(';') {
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
            println!("cargo:rustc-link-lib=static:+whole-archive=z");
            if std::env::var("OPENCV_LINK_PATHS").unwrap_or_default().contains("vcpkg") {
                std::env::var("OPENCV_LINK_LIBS").unwrap().split(',').for_each(|lib| println!("cargo:rustc-link-lib=static={}", lib.trim()));
            } else {
                std::env::var("OPENCV_LINK_LIBS").unwrap().split(',').for_each(|lib| println!("cargo:rustc-link-lib={}", lib.trim()));
            }
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
