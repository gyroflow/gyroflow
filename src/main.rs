#![recursion_limit="4096"]
//#![windows_subsystem = "windows"]

use cpp::*;
use qmetaobject::*;
use std::cell::RefCell;

pub mod core;
pub mod controller;
pub mod rendering;
pub mod ui { pub mod components { pub mod TimelineGyroChart; } }

use crate::core::{lens_profile::LensProfile, smoothing::*};
use ui::components::TimelineGyroChart::TimelineGyroChart;

// TODO: frame readout time from metadata for gopro and insta360 
// TODO: PR to ahrs-rs
// TODO: Move thread pool to core
// TODO: separate controller into multiple files
// TODO: analyze every n-th frame
    
// TODO: warning when no lens profile loaded
// TODO: negative rolling shutter values (bottom to top)
// TODO: more smoothing algorithms
// TODO: adaptive zoom
// TODO: output size and correctly fit the undistortion in it
// TODO: UI fixes, editing offset, double animations etc
// TODO: output size aspect ratio lock icon
// TODO: default lens profile and FOV
// TODO: exporting .gyroflow
// TODO: saving settings, storage
// TODO: exporting .gyroflow file
// TODO: loading .gyroflow file (test)
// TODO: Calibrator
// TODO: -- auto upload of lens profiles to a central database (with a checkbox)
// TODO: -- Save camera model with calibration and later load lens profile automatically
// TODO: UI: activeFocus indicators
// TODO: languages
// TODO: something is wrong with Complementary integrator
// TODO: lens profile param adjustment
// TODO: confirm when render output already exists
// TODO: better output name, strip extension
// TODO: wgpu undistortion add support for different plane types
// TODO: add lens distortion back after stabilization
// TODO: video rotation
// TODO: hyperlapse mode

// DETAILS:
// TODO: When syncing it shouldn't be possible to change any sync settings, but it should be possible to cancel
// TODO: When rendering, it should be possible to "minimize" the status and continue to work. Also it should be possible to cancel at any time (and this should produce correct file)
// TODO: It shouldn't be possible to do syncing without a lens profile
// TODO: Recompute undistortion only for the trimmed range
// TODO: drop mutliple files (video, lens profile, gyro data) at once
// TODO: add elapsed and remaining times when rendering

qrc!(rsrc,
    "/" {
        "src/ui/main_window.qml",
        "src/ui/App.qml",
        "src/ui/VideoArea.qml",

        "src/ui/menu/Advanced.qml",
        "src/ui/menu/Export.qml",
        "src/ui/menu/LensProfile.qml",
        "src/ui/menu/MotionData.qml",
        "src/ui/menu/Stabilization.qml",
        "src/ui/menu/Synchronization.qml",
        "src/ui/menu/VideoInformation.qml",
        "src/ui/components/BasicText.qml",
        "src/ui/components/Button.qml",
        "src/ui/components/CheckBox.qml",
        "src/ui/components/CheckBoxWithContent.qml",
        "src/ui/components/ComboBox.qml",
        "src/ui/components/DropdownChevron.qml",
        "src/ui/components/DropTarget.qml",
        "src/ui/components/DropTargetRect.qml",
        "src/ui/components/Ease.qml",
        "src/ui/components/Hr.qml",
        "src/ui/components/Label.qml",
        "src/ui/components/LinkButton.qml",
        "src/ui/components/LoaderOverlay.qml",
        "src/ui/components/MenuItem.qml",
        "src/ui/components/Modal.qml",
        "src/ui/components/NumberField.qml",
        "src/ui/components/Popup.qml",
        "src/ui/components/ResizablePanel.qml",
        "src/ui/components/SearchField.qml",
        "src/ui/components/SidePanel.qml",
        "src/ui/components/Slider.qml",
        "src/ui/components/SplitButton.qml",
        "src/ui/components/TableList.qml",
        "src/ui/components/TextField.qml",
        "src/ui/components/Timeline.qml",
        "src/ui/components/TimelineAxisButton.qml",
        "src/ui/components/TimelineRangeIndicator.qml",
        "src/ui/components/TimelineSyncPoint.qml",
        "src/ui/components/ToolTip.qml",
        
        "resources/icon.png",
        "resources/logo_black.svg",
        "resources/logo_white.svg",
        "resources/icons/index.theme",
        "resources/icons/svg/bin.svg",
        "resources/icons/svg/chart.svg",
        "resources/icons/svg/chevron-down.svg",
        "resources/icons/svg/chevron-left.svg",
        "resources/icons/svg/chevron-right.svg",
        "resources/icons/svg/file-empty.svg",
        "resources/icons/svg/gyroflow.svg",
        "resources/icons/svg/info.svg",
        "resources/icons/svg/lens.svg",
        "resources/icons/svg/lock.svg",
        "resources/icons/svg/pause.svg",
        "resources/icons/svg/pencil.svg",
        "resources/icons/svg/play.svg",
        "resources/icons/svg/plus.svg",
        "resources/icons/svg/save.svg",
        "resources/icons/svg/search.svg",
        "resources/icons/svg/settings.svg",
        "resources/icons/svg/sound-mute.svg",
        "resources/icons/svg/sound.svg",
        "resources/icons/svg/spinner.svg",
        "resources/icons/svg/sync.svg",
        "resources/icons/svg/unlocked.svg",
        "resources/icons/svg/video.svg",
    }
);

cpp! {{
    #include <QQuickStyle>
    #include <QQuickWindow>
    #include <QIcon>

    #include "src/ui_live_reload.cpp"
}}

#[derive(Default, QObject)]
struct Theme { 
    base: qt_base_class!(trait QObject), 
    set_theme: qt_method!(fn(theme: String)),

    engine_ptr: Option<*mut QmlEngine>
}
impl Theme {
    pub fn set_theme(&self, theme: String) {
        let engine = unsafe { &mut *(self.engine_ptr.unwrap()) };

        engine.set_property("styleFont".into(), QVariant::from(QString::from("Kozuka")));

        match theme.as_str() {
            "dark" => {
                engine.set_property("style"                 .into(), QVariant::from(QString::from("dark")));
                engine.set_property("styleBackground"       .into(), QVariant::from(QString::from("#272727")));
                engine.set_property("styleBackground2"      .into(), QVariant::from(QString::from("#202020")));
                engine.set_property("styleButtonColor"      .into(), QVariant::from(QString::from("#2d2d2d")));
                engine.set_property("styleTextColor"        .into(), QVariant::from(QString::from("#ffffff")));
                engine.set_property("styleAccentColor"      .into(), QVariant::from(QString::from("#76baed")));
                engine.set_property("styleVideoBorderColor" .into(), QVariant::from(QString::from("#313131")));
                engine.set_property("styleTextColorOnAccent".into(), QVariant::from(QString::from("#000000")));
                engine.set_property("styleHrColor"          .into(), QVariant::from(QString::from("#323232")));
                engine.set_property("stylePopupBorder"      .into(), QVariant::from(QString::from("#141414")));
                engine.set_property("styleSliderHandle"     .into(), QVariant::from(QString::from("#454545")));
                engine.set_property("styleSliderBackground" .into(), QVariant::from(QString::from("#9a9a9a")));
                engine.set_property("styleHighlightColor"   .into(), QVariant::from(QString::from("#10ffffff")));
            },
            "light" => {
                engine.set_property("style"                 .into(), QVariant::from(QString::from("light")));
                engine.set_property("styleBackground"       .into(), QVariant::from(QString::from("#f9f9f9")));
                engine.set_property("styleBackground2"      .into(), QVariant::from(QString::from("#f3f3f3")));
                engine.set_property("styleButtonColor"      .into(), QVariant::from(QString::from("#fbfbfb")));
                engine.set_property("styleTextColor"        .into(), QVariant::from(QString::from("#111111")));
                engine.set_property("styleAccentColor"      .into(), QVariant::from(QString::from("#116cad")));
                engine.set_property("styleVideoBorderColor" .into(), QVariant::from(QString::from("#d5d5d5")));
                engine.set_property("styleTextColorOnAccent".into(), QVariant::from(QString::from("#ffffff")));
                engine.set_property("styleHrColor"          .into(), QVariant::from(QString::from("#e5e5e5")));
                engine.set_property("stylePopupBorder"      .into(), QVariant::from(QString::from("#d5d5d5")));
                engine.set_property("styleSliderHandle"     .into(), QVariant::from(QString::from("#c2c2c2")));
                engine.set_property("styleSliderBackground" .into(), QVariant::from(QString::from("#cdcdcd")));
                engine.set_property("styleHighlightColor"   .into(), QVariant::from(QString::from("#10000000")));
            },
            _ => { }
        }
    }
}

fn main() {
    rsrc();
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
    engine.set_property("dpiScale".into(), QVariant::from(1.0));
    engine.set_object_property("controller".into(), ctlpinned);
    engine.set_object_property("theme".into(), themepinned);
    theme.borrow_mut().engine_ptr = Some(&mut engine as *mut _);
    theme.borrow().set_theme("dark".into());

    // Get camera profiles list
    let lens_profiles = QVariantList::from_iter(LensProfile::get_profiles_list().unwrap_or_default().into_iter().map(QString::from));
    engine.set_property("lensProfilesList".into(), QVariant::from(lens_profiles));

    // Get smoothing algorithms
    let algorithms = QVariantList::from_iter(get_smoothing_algorithms().into_iter().map(|x| QString::from(x.get_name())));
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
    });

    engine.exec();
}
