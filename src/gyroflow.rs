#![recursion_limit="4096"]
//#![windows_subsystem = "windows"]

use cpp::*;
use qmetaobject::*;
use std::cell::RefCell;

pub mod core;
pub mod controller;
pub mod rendering;
pub mod resources;
pub mod ui { pub mod theme; pub mod components { pub mod TimelineGyroChart; } }

use crate::core::{lens_profile::LensProfile, smoothing::*};
use ui::components::TimelineGyroChart::TimelineGyroChart;
use ui::theme::Theme;

// Things to do before first public preview:
// - Move thread pool to core
// - Separate controller into multiple files and clean it up
// - Fix mutex locking for too long
// - Setup CI for packaging for Windows
// - Setup CI for packaging for Mac
// - UI fixes, editing offset, double animations etc
// - Fix ffmpeg GPU acceleration detection and test with different graphic cards
// - Review offsets interpolation code, it doesn't seem to behave correctly with large offsets
// - Some basic error handling, check for all unwrap()'s
// - Add font using QFontDatabase

// TODO: more smoothing algorithms
// TODO: adaptive zoom

// TODO: exporting and loading .gyroflow
// TODO: output size and correctly fit the undistortion in it
// TODO: video rotation
// TODO: default lens profile
// TODO: saving settings, storage
// TODO: Calibrator
// TODO: -- auto upload of lens profiles to a central database (with a checkbox)
// TODO: -- Save camera model with calibration and later load lens profile automatically
// TODO: UI: activeFocus indicators
// TODO: languages
// TODO: something is wrong with Complementary integrator
// TODO: lens profile param adjustment
// TODO: wgpu undistortion add support for different plane types
// TODO: add lens distortion back after stabilization
// TODO: hyperlapse mode
// TODO: Setup CI for packaging for Linux
// TODO: Setup CI for packaging for Android
// TODO: Setup CI for packaging for iOS
// TODO: drop mutliple files at once (video, lens profile, gyro data)
// TODO: add elapsed and remaining times when rendering
// TODO: add vertical labels and scale to gyro chart
// TODO: When rendering, it should be possible to "minimize" the status and continue to work. 

cpp! {{
    #include <QQuickStyle>
    #include <QQuickWindow>
    #include <QtGui/QGuiApplication>
    #include <QIcon>

    #include "src/ui_live_reload.cpp"

    #ifdef Q_OS_ANDROID
    #   include <QtCore/private/qandroidextras_p.h>
    #endif
}}

pub fn entry() {
    crate::resources::rsrc();
    qml_video_rs::register_qml_types();
    qml_register_type::<TimelineGyroChart>(cstr::cstr!("Gyroflow"), 1, 0, cstr::cstr!("TimelineGyroChart"));

    // return rendering::test();

    // let icons_path = QString::from(format!("{}/resources/icons/", env!("CARGO_MANIFEST_DIR")));
    let icons_path = QString::from(":/resources/icons/");
    cpp!(unsafe [icons_path as "QString"] {
        QQuickStyle::setStyle("Material");
        QIcon::setThemeName(QStringLiteral("Gyroflow"));
        QIcon::setThemeSearchPaths(QStringList() << icons_path);
        
        // QQuickWindow::setGraphicsApi(QSGRendererInterface::OpenGL);
        // QQuickWindow::setGraphicsApi(QSGRendererInterface::Vulkan);
    });

    let ctl = RefCell::new(controller::Controller::new());
    let ctlpinned = unsafe { QObjectPinned::new(&ctl) };

    let theme = RefCell::new(Theme::default());
    let themepinned = unsafe { QObjectPinned::new(&theme) };

    let mut engine = QmlEngine::new();
    let dpi = cpp!(unsafe[] -> f64 as "double" { return QGuiApplication::primaryScreen()->logicalDotsPerInch() / 96.0; });
    engine.set_property("dpiScale".into(), QVariant::from(dpi));
    engine.set_object_property("controller".into(), ctlpinned);
    engine.set_object_property("theme".into(), themepinned);
    theme.borrow_mut().engine_ptr = Some(&mut engine as *mut _);
    theme.borrow().set_theme("dark".into());

    // Get camera profiles list
    let lens_profiles: QVariantList = LensProfile::get_profiles_list().unwrap_or_default().into_iter().map(QString::from).collect();
    engine.set_property("lensProfilesList".into(), QVariant::from(lens_profiles));

    // Get smoothing algorithms
    let algorithms: QVariantList = get_smoothing_algorithms().into_iter().map(|x| QString::from(x.get_name())).collect();
    engine.set_property("smoothingAlgorithms".into(), QVariant::from(algorithms));

    let engine_ptr = engine.cpp_ptr();

    // Load main UI
    let live_reload = false;
    if !live_reload {
        engine.load_file("qrc:/src/ui/main_window.qml".into());
    } else {
        engine.load_file(format!("{}/src/ui/main_window.qml", env!("CARGO_MANIFEST_DIR")).into());
        let ui_path = QString::from(format!("{}/src/ui", env!("CARGO_MANIFEST_DIR")));
        cpp!(unsafe [engine_ptr as "QQmlApplicationEngine *", ui_path as "QString"] { init_live_reload(engine_ptr, ui_path); });
    }

    cpp!(unsafe [engine_ptr as "QQmlApplicationEngine *"] {
        qobject_cast<QQuickWindow *>(engine_ptr->rootObjects().first())->setIcon(QIcon(":/resources/icon.png"));
        #ifdef Q_OS_ANDROID
            QtAndroidPrivate::requestPermission(QtAndroidPrivate::Storage).result();
        #endif
    });

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
