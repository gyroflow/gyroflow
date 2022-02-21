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
pub mod resources;
pub mod ui { pub mod ui_tools; pub mod components { pub mod TimelineGyroChart; } }
pub mod qt_gpu { pub mod qrhi_undistort; }

use ui::components::TimelineGyroChart::TimelineGyroChart;
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
    unsafe { winapi::um::wincon::AttachConsole(winapi::um::wincon::ATTACH_PARENT_PROCESS); }

    let _ = util::install_crash_handler();
    util::init_logging();

    crate::resources::rsrc();
    qml_video_rs::register_qml_types();
    qml_register_type::<TimelineGyroChart>(cstr::cstr!("Gyroflow"), 1, 0, cstr::cstr!("TimelineGyroChart"));

    // rendering::set_gpu_type_from_name("nvidia");
    // rendering::test();
    // return;

    let icons_path = if ui_live_reload {
        QString::from(format!("{}/resources/icons/", env!("CARGO_MANIFEST_DIR")))
    } else {
        QString::from(":/resources/icons/")
    };
    cpp!(unsafe [icons_path as "QString"] {
        QGuiApplication::setOrganizationName("Gyroflow");
        QGuiApplication::setOrganizationDomain("gyroflow.xyz");
        QGuiApplication::setApplicationName("Gyroflow");

        QQuickStyle::setStyle("Material");
        QIcon::setThemeName(QStringLiteral("Gyroflow"));
        QIcon::setThemeSearchPaths(QStringList() << icons_path);

        // QQuickWindow::setGraphicsApi(QSGRendererInterface::OpenGL);
        // QQuickWindow::setGraphicsApi(QSGRendererInterface::Vulkan);
    });
    // if cfg!(target_os = "android") {
    //     cpp!(unsafe [] { QQuickWindow::setGraphicsApi(QSGRendererInterface::Vulkan); });
    // }

    if cfg!(target_os = "android") || cfg!(target_os = "ios") {
        MDKVideoItem::setGlobalOption("MDK_KEY", "B75BC812C266C3E2D967840494C8866773E4E5FC596729F7D9895BFB2DB3B9AE2515F306FBF29BF20290E1093E9A5B5796B778F866F5F631831\
            0431F1E34810348A437EDC2663C1D26987BFB6B37799871E4E984201D0790A0FB349D41DCCEAE15E8C6B790A89ADA30C4B6EB323303B0603B3A2BBF50C294456F377CA8FEF103");
    } else {
        MDKVideoItem::setGlobalOption("MDK_KEY", "47FA7B212D5FF2F649A245E6D8DC2D88BAB67C208282CB3E2DEB95B9B4F9EC575102303FB92448ED49454E027A31B48ED08824EB904B58F693AD\
            B52FA63A4008B80584DE2D5F0D09B65DBA192723D277B8B67447FBF0A4584184E2659155D95CFBEB08626CBE3C94416B2FC50B1FA1201AA7381CE3E85DF3F3BF9BCB59677808");
    }

    let ctl = RefCell::new(controller::Controller::new());
    let ctlpinned = unsafe { QObjectPinned::new(&ctl) };

    let ui_tools = RefCell::new(UITools::default());
    let ui_tools_pinned = unsafe { QObjectPinned::new(&ui_tools) };

    let mut engine = QmlEngine::new();
    let dpi = cpp!(unsafe[] -> f64 as "double" { return QGuiApplication::primaryScreen()->logicalDotsPerInch() / 96.0; });
    engine.set_property("dpiScale".into(), QVariant::from(dpi));
    engine.set_property("version".into(), QString::from(util::get_version()).into());
    engine.set_property("isOpenGl".into(), QVariant::from(false));
    engine.set_object_property("main_controller".into(), ctlpinned);
    engine.set_object_property("ui_tools".into(), ui_tools_pinned);
    ui_tools.borrow_mut().engine_ptr = Some(&mut engine as *mut _);
    ui_tools.borrow().set_theme("dark".into());

    // Get smoothing algorithms
    engine.set_property("smoothingAlgorithms".into(), QVariant::from(ctl.borrow().get_smoothing_algs()));

    let engine_ptr = engine.cpp_ptr();

    // Load main UI
    if !ui_live_reload {
        engine.load_file("qrc:/src/ui/main_window.qml".into());
    } else {
        engine.load_file(format!("{}/src/ui/main_window.qml", env!("CARGO_MANIFEST_DIR")).into());
        let ui_path = QString::from(format!("{}/src/ui", env!("CARGO_MANIFEST_DIR")));
        cpp!(unsafe [engine_ptr as "QQmlApplicationEngine *", ui_path as "QString"] { init_live_reload(engine_ptr, ui_path); });
    }

    cpp!(unsafe [] {
        #ifdef Q_OS_ANDROID
            QtAndroidPrivate::requestPermission(QtAndroidPrivate::Storage).result();
        #endif
    });

    let is_opengl = util::is_opengl();
    engine.set_property("isOpenGl".into(), QVariant::from(is_opengl));
    ctl.borrow_mut().stabilizer.params.write().framebuffer_inverted = is_opengl;

    rendering::init().unwrap();

    if let Some(name) = core::gpu::initialize_contexts() {
        rendering::set_gpu_type_from_name(&name);
    }

    engine.exec();
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
