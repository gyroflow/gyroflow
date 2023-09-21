// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

#![recursion_limit="4096"]
#![windows_subsystem = "windows"]

use cpp::*;
use qmetaobject::*;
use qml_video_rs::video_item::MDKVideoItem;
use std::cell::RefCell;

pub use gyroflow_core as core;
pub mod util;
pub mod controller;
pub mod rendering;
pub mod external_sdk;
mod cli;
mod resources;
#[cfg(not(compiled_qml))]
mod resources_qml;
pub mod ui { pub mod ui_tools; pub mod components { pub mod TimelineGyroChart; pub mod TimelineKeyframesView; pub mod FrequencyGraph; } }
pub mod qt_gpu { pub mod qrhi_undistort; }

use ui::components::TimelineGyroChart::TimelineGyroChart;
use ui::components::TimelineKeyframesView::TimelineKeyframesView;
use ui::components::FrequencyGraph::FrequencyGraph;
use ui::ui_tools::UITools;

cpp! {{
    #include <QQuickStyle>
    #include <QQuickWindow>
    #include <QQmlContext>
    #include <QtGui/QGuiApplication>
    #include <QIcon>

    #include "src/ui_live_reload.cpp"

    #ifdef Q_OS_ANDROID
    #   include <QtCore/private/qandroidextras_p.h>
    #endif
}}

fn entry() {
    let ui_live_reload = false;

    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::System::Console::*;
        if !AttachConsole(ATTACH_PARENT_PROCESS).is_ok() && cli::will_run_in_console() {
            let _ = AllocConsole();
        }
    }

    let _ = util::install_crash_handler();
    util::init_logging();
    util::update_rlimit();
    util::set_android_context();
    log_panics::init();

    cpp!(unsafe [] {
        qApp->setOrganizationName("Gyroflow");
        qApp->setOrganizationDomain("gyroflow.xyz");
        qApp->setApplicationName("Gyroflow");

        QMessageLogger("", 0, "main").debug(QLoggingCategory("gyroflow")) << "Qt version:" << qVersion();
    });
    ::log::debug!("Gyroflow {}", util::get_version());

    let mut open_file = String::new();
    if cli::run(&mut open_file) {
        return;
    }

    if cfg!(compiled_qml) {
        // For some reason on some devices QML detects that debugger is connected and fails to load pre-compiled qml files
        cpp!(unsafe [] { qputenv("QML_FORCE_DISK_CACHE", "1"); });
    }

    crate::resources::rsrc();
    #[cfg(not(compiled_qml))]
    crate::resources_qml::rsrc_qml();

    qml_video_rs::register_qml_types();
    qml_register_type::<TimelineGyroChart>(cstr::cstr!("Gyroflow"), 1, 0, cstr::cstr!("TimelineGyroChart"));
    qml_register_type::<TimelineKeyframesView>(cstr::cstr!("Gyroflow"), 1, 0, cstr::cstr!("TimelineKeyframesView"));
    qml_register_type::<FrequencyGraph>(cstr::cstr!("Gyroflow"), 1, 0, cstr::cstr!("FrequencyGraph"));

    let icons_path = if ui_live_reload {
        QString::from(format!("{}/resources/icons/", env!("CARGO_MANIFEST_DIR")))
    } else {
        QString::from(":/resources/icons/")
    };
    cpp!(unsafe [icons_path as "QString"] {
        QQuickStyle::setStyle("Material");
        QIcon::setThemeName(QStringLiteral("Gyroflow"));
        QIcon::setThemeSearchPaths(QStringList() << icons_path);

        #ifdef Q_OS_ANDROID
            // QQuickWindow::setGraphicsApi(QSGRendererInterface::Vulkan);
            int av_jni_set_java_vm(void *vm, void *log_ctx);
            av_jni_set_java_vm(QJniEnvironment::javaVM(), nullptr);
        #endif

        // QQuickWindow::setGraphicsApi(QSGRendererInterface::OpenGL);
        // QQuickWindow::setGraphicsApi(QSGRendererInterface::Vulkan);
    });

    util::save_exe_location();
    let sdk_path = external_sdk::SDK_PATH.as_ref().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
    ::log::debug!("Executable path: {:?}", gyroflow_core::util::get_setting("exeLocation"));
    ::log::debug!("SDK path: {:?}", sdk_path);

    //crate::core::util::rename_calib_videos();

    if cfg!(any(target_os = "android", target_os = "ios")) {
        MDKVideoItem::setGlobalOption("MDK_KEY", "B75BC812C266C3E2D967840494C8866773E4E5FC596729F7D9895BFB2DB3B9AE2515F306FBF29BF20290E1093E9A5B5796B778F866F5F631831\
            0431F1E34810348A437EDC2663C1D26987BFB6B37799871E4E984201D0790A0FB349D41DCCEAE15E8C6B790A89ADA30C4B6EB323303B0603B3A2BBF50C294456F377CA8FEF103");
    } else {
        MDKVideoItem::setGlobalOption("MDK_KEY", "47FA7B212D5FF2F649A245E6D8DC2D88BAB67C208282CB3E2DEB95B9B4F9EC575102303FB92448ED49454E027A31B48ED08824EB904B58F693AD\
            B52FA63A4008B80584DE2D5F0D09B65DBA192723D277B8B67447FBF0A4584184E2659155D95CFBEB08626CBE3C94416B2FC50B1FA1201AA7381CE3E85DF3F3BF9BCB59677808");
        MDKVideoItem::setGlobalOption("plugins", "mdk-braw:mdk-r3d");
    }

    if cfg!(target_os = "linux") {
        // Init wgpu before Qt because of a bug in `khronos-egl`
        gyroflow_core::gpu::wgpu::WgpuWrapper::list_devices();
    }

    let _ = external_sdk::cleanup();

    let ctl = RefCell::new(controller::Controller::new());
    let ctlpinned = unsafe { QObjectPinned::new(&ctl) };

    let ui_tools = RefCell::new(UITools::default());
    let ui_tools_pinned = unsafe { QObjectPinned::new(&ui_tools) };

    let rq = RefCell::new(rendering::render_queue::RenderQueue::new(ctl.borrow().stabilizer.clone()));
    let rqpinned = unsafe { QObjectPinned::new(&rq) };

    let fs = RefCell::new(controller::Filesystem::default());
    let fspinned = unsafe { QObjectPinned::new(&fs) };

    util::set_url_catcher(fspinned.get_or_create_cpp_object());
    util::register_url_handlers();

    let mut engine = QmlEngine::new();
    util::catch_qt_file_open(|url| {
        engine.set_property("openFileOnStart".into(), url.into());
    });
    let mut dpi = cpp!(unsafe[] -> f64 as "double" { return QGuiApplication::primaryScreen()->logicalDotsPerInch() / 96.0; });
    if cfg!(target_os = "android") { dpi *= 0.85; }
    engine.set_property("dpiScale".into(), QVariant::from(dpi));
    engine.set_property("version".into(), QString::from(util::get_version()).into());
    engine.set_property("graphics_api".into(), util::qt_graphics_api().into());
    engine.set_object_property("main_controller".into(), ctlpinned);
    engine.set_object_property("ui_tools".into(), ui_tools_pinned);
    engine.set_object_property("render_queue".into(), rqpinned);
    engine.set_object_property("filesystem".into(), fspinned);
    {
        let mut ui = ui_tools.borrow_mut();
        ui.engine_ptr = Some(&mut engine as *mut _);
        ui.set_theme("dark".into());
    }

    // Get smoothing algorithms
    engine.set_property("smoothingAlgorithms".into(), QVariant::from(ctl.borrow().get_smoothing_algs()));

    let engine_ptr = engine.cpp_ptr();

    // Load main UI
    if !ui_live_reload {
        use std::path::PathBuf;
        // Try to load from disk first
        let path = (|| -> Option<String> {
            let path = if cfg!(any(target_os = "macos", target_os = "ios")) {
                PathBuf::from("../Resources/ui/main_window.qml")
            } else {
                PathBuf::from("./ui/main_window.qml")
            };
            let final_path = std::env::current_exe().ok()?.parent()?.join(path);
            if final_path.exists() {
                Some(String::from(final_path.to_str()?))
            } else {
                None
            }
        })();
        if let Some(path) = path {
            engine.load_file(path.into());
        } else {
            // Load from resources
            engine.load_url(QString::from("qrc:/src/ui/main_window.qml").into());
        }
    } else {
        engine.load_file(format!("{}/src/ui/main_window.qml", env!("CARGO_MANIFEST_DIR")).into());
        let ui_path = QString::from(format!("{}/src/ui", env!("CARGO_MANIFEST_DIR")));
        cpp!(unsafe [engine_ptr as "QQmlApplicationEngine *", ui_path as "QString"] { init_live_reload(engine_ptr, ui_path); });
    }

    cpp!(unsafe [] {
        #ifdef Q_OS_ANDROID
            QtAndroidPrivate::requestPermission("android.permission.READ_EXTERNAL_STORAGE").result();
            QtAndroidPrivate::requestPermission("android.permission.WRITE_EXTERNAL_STORAGE").result();
            QtAndroidPrivate::requestPermission("android.permission.READ_MEDIA_VIDEO").result();
        #endif
    });

    ctl.borrow_mut().stabilizer.params.write().framebuffer_inverted = util::is_opengl();

    rendering::init().unwrap();

    engine.set_property("openFileOnStart".into(), QUrl::from(QString::from(gyroflow_core::filesystem::path_to_url(&open_file))).into());

    engine.set_property("defaultInitializedDevice".into(), QString::default().into());
    if let Some((name, list_name)) = core::gpu::initialize_contexts() {
        rendering::set_gpu_type_from_name(&name);
        engine.set_property("defaultInitializedDevice".into(), QString::from(list_name).into());
    }

    engine.exec();

    util::unregister_url_handlers();
}


#[no_mangle]
#[cfg(target_os = "android")]
pub extern fn main(_argc: i32, _argv: *mut *mut i8) -> i32 {
    entry();
    0
}

#[cfg(not(target_os = "android"))]
fn main() {
    entry();
}
