// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

#![recursion_limit="4096"]
#![windows_subsystem = "windows"]

use cpp::*;
use qmetaobject::*;
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

// TODO: use quaternions for finding offset, not gyro samples
// TODO: fix loader that stays on after load sometimes

// TODO: Batch processing when loaded multiple files
// TODO: wgpu convert to using textures
// TODO: dragging numbers on the numberfield left and right
// TODO: Review offsets interpolation code, it doesn't seem to behave correctly with large offsets
// TODO: smoothing presets
// TODO: cli interface
// TODO: Calibrator: Allow for multiple zoom values, could be interpolated later (Sony)
// TODO: UI: activeFocus indicators
// TODO: timeline panning
// TODO: add lens distortion back after stabilization
// TODO: hyperlapse mode
// TODO: video speed 
// TODO: export framerate
// TODO: export pixel format conversion (ComboBox in the UI)
// TODO: Setup CI for packaging for Android
// TODO: Setup CI for packaging for iOS
// TODO: drop mutliple files at once (video, lens profile, gyro data)
// TODO: add elapsed and remaining times when rendering
// TODO: add vertical labels and scale lines to gyro chart
// TODO: render queue
// TODO: When rendering, it should be possible to "minimize" the status and let it render in render queue
// TODO: keyframes for stabilization params
// TODO: detect imu orientation automatically, basically try all combinations for a closest match to OF
// TODO: mask for optical flow
// TODO: Add cache for the undistortion if the video is not playing
// TODO: OpenFX plugin
// TODO: Adobe plugin
// TODO: exporting .gyroflow: include output settings and allow user to choose thin or full file
// TODO: preview resolution calculation is wrong when setting the output size to 1280x720 and the source size is 4k
// TODO: support GoPro's superview lens correction
// TODO: Figure out Sony lens distortion parameters
// TODO: save panel sizes, menu opened states and window dimensions
// TODO: audio slightly off sync when using exporting trimmed video
// TODO: when optical flow data already exists, using "Auto sync here" doesn't show the loading thing

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

    let log_config = simplelog::ConfigBuilder::new()
        .add_filter_ignore_str("mp4parse")
        .add_filter_ignore_str("wgpu")
        .add_filter_ignore_str("naga")
        .add_filter_ignore_str("akaze")
        .add_filter_ignore_str("ureq")
        .add_filter_ignore_str("rustls")
        .build();

    #[cfg(target_os = "android")]
    simplelog::WriteLogger::init(simplelog::LevelFilter::Debug, log_config, util::AndroidLog::default()).unwrap();

    #[cfg(not(target_os = "android"))]
    simplelog::TermLogger::init(simplelog::LevelFilter::Debug, log_config, simplelog::TerminalMode::Mixed, simplelog::ColorChoice::Auto).unwrap();

    qmetaobject::log::init_qt_to_rust();

    crate::resources::rsrc();
    qml_video_rs::register_qml_types();
    qml_register_type::<TimelineGyroChart>(cstr::cstr!("Gyroflow"), 1, 0, cstr::cstr!("TimelineGyroChart"));

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

    if cfg!(target_os = "android") {
        cpp!(unsafe [] { QQuickWindow::setGraphicsApi(QSGRendererInterface::Vulkan); });
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
