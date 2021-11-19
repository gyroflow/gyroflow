fn main() {
    let qt_include_path = std::env::var("DEP_QT_INCLUDE_PATH").unwrap();
    let qt_library_path = std::env::var("DEP_QT_LIBRARY_PATH").unwrap();

    #[allow(unused_mut)]
    let mut config = cpp_build::Config::new();

    if cfg!(target_os = "macos") {
        config.flag("-F");
        config.flag(&qt_library_path);
        println!("cargo:rustc-link-lib=z");
        println!("cargo:rustc-link-lib=bz2");
        println!("cargo:rustc-link-lib=xml2");
        println!("cargo:rustc-link-lib=framework=VideoToolbox");
        println!("cargo:rustc-link-lib=framework=CoreAudio");
    }

    let mut public_include = |name| { config.include(format!("{}/{}", qt_include_path, name)); };
    public_include("QtCore");
    public_include("QtGui");
    public_include("QtQuick");
    public_include("QtQml");
    public_include("QtQuickControls2");

    config
        .include(&qt_include_path)
        .flag_if_supported("-std=c++17")
        .flag_if_supported("/std:c++17")
        .flag_if_supported("/Zc:__cplusplus")
        .build("src/main.rs");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS");
    match target_os.as_ref().map(|x| &**x) {
        Ok("android") => {
            println!("cargo:rustc-link-search=D:\\Programy\\Qt\\6.2.1\\android_arm64_v8a\\lib");
            println!("cargo:rustc-link-lib=Qt6Network_arm64-v8a");
            println!("cargo:rustc-link-lib=Qt6OpenGL_arm64-v8a");
            println!("cargo:rustc-link-lib=Qt6QmlModels_arm64-v8a");
            println!("cargo:rustc-link-lib=Qt6QuickTemplates2_arm64-v8a");
            println!("cargo:rustc-link-lib=android");
            println!("cargo:rustc-link-lib=OpenSLES");
            println!("cargo:rustc-link-lib=GLESv2");
            println!("cargo:rustc-link-lib=EGL");
        },
        Ok("macos") => {
            
        }
        Ok("windows") => {
            let mut res = winres::WindowsResource::new();
            res.set_icon("resources/app_icon.ico");
            res.compile().unwrap();
        }
        tos => panic!("unknown target os {:?}!", tos)
    }

}
