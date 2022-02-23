// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use qmetaobject::*;
use cpp::*;
use std::cell::RefCell;
use crate::controller::Controller;
use crate::util;

#[cfg(target_os = "windows")]
use windows::Win32::{ Foundation::HWND, UI::Shell::{ ITaskbarList4, TaskbarList }, System::Com::{ CoInitializeEx, CoCreateInstance, CLSCTX_ALL, COINIT_MULTITHREADED } };

cpp! {{
    #include <QTranslator>
}}

#[derive(Default, QObject)]
pub struct UITools { 
    base: qt_base_class!(trait QObject), 
    set_theme: qt_method!(fn(&self, theme: String)),
    set_language: qt_method!(fn(&self, lang_id: QString)),
    get_default_language: qt_method!(fn(&self) -> QString),
    set_scaling: qt_method!(fn(&self, dpiScale: f64)),
    init_calibrator: qt_method!(fn(&mut self)),
    set_icon: qt_method!(fn(&mut self, wnd: QJSValue)),
    set_progress: qt_method!(fn(&self, progress: f64)),

    language_changed: qt_signal!(),

    calibrator_ctl: Option<RefCell<Controller>>,

    #[cfg(target_os = "windows")]
    taskbar: Option<ITaskbarList4>,

    main_window_handle: Option<isize>,

    pub engine_ptr: Option<*mut QmlEngine>
}
impl UITools {
    pub fn set_language(&self, lang_id: QString) {
        if let Some(engine) = self.engine_ptr {
            let engine = unsafe { &mut *(engine) };
            let engine_ptr = engine.cpp_ptr();
            cpp!(unsafe [engine_ptr as "QQmlEngine *", lang_id as "QString"] {
                static QTranslator translator;
                QCoreApplication::removeTranslator(&translator);
                if (lang_id != "en") {
                    if (translator.load(":/resources/translations/" + lang_id + ".qm")) {
                        qApp->setLayoutDirection((lang_id == "ar" || lang_id == "fa" || lang_id == "he")? Qt::RightToLeft : Qt::LeftToRight);
                        QCoreApplication::installTranslator(&translator);
                    }
                }

                engine_ptr->retranslate();
            });
            self.language_changed();
        }
    }
    pub fn get_default_language(&self) -> QString {
        cpp!(unsafe [] -> QString as "QString" {
            QString lang  = QLocale::system().name();
            QString lang2 = lang.mid(0, 2);
            if (QFile::exists(":/resources/translations/" + lang + ".qm")) return lang;
            if (QFile::exists(":/resources/translations/" + lang2 + ".qm")) return lang2;
            return "en";
        })
    }

    pub fn set_theme(&self, theme: String) {
        if let Some(engine) = self.engine_ptr {
            let engine = unsafe { &mut *(engine) };
        
            cpp!(unsafe [] { auto f = QGuiApplication::font(); f.setFamily("Arial"); QGuiApplication::setFont(f); });
            engine.set_property("styleFont".into(), QString::from("Arial").into());

            match theme.as_str() {
                "dark" => {
                    engine.set_property("style"                 .into(), QString::from("dark").into());
                    engine.set_property("styleBackground"       .into(), QString::from("#272727").into());
                    engine.set_property("styleBackground2"      .into(), QString::from("#202020").into());
                    engine.set_property("styleButtonColor"      .into(), QString::from("#2d2d2d").into());
                    engine.set_property("styleTextColor"        .into(), QString::from("#ffffff").into());
                    engine.set_property("styleAccentColor"      .into(), QString::from("#76baed").into());
                    engine.set_property("styleVideoBorderColor" .into(), QString::from("#313131").into());
                    engine.set_property("styleTextColorOnAccent".into(), QString::from("#000000").into());
                    engine.set_property("styleHrColor"          .into(), QString::from("#323232").into());
                    engine.set_property("stylePopupBorder"      .into(), QString::from("#141414").into());
                    engine.set_property("styleSliderHandle"     .into(), QString::from("#454545").into());
                    engine.set_property("styleSliderBackground" .into(), QString::from("#9a9a9a").into());
                    engine.set_property("styleHighlightColor"   .into(), QString::from("#10ffffff").into());
                },
                "light" => {
                    engine.set_property("style"                 .into(), QString::from("light").into());
                    engine.set_property("styleBackground"       .into(), QString::from("#f9f9f9").into());
                    engine.set_property("styleBackground2"      .into(), QString::from("#f3f3f3").into());
                    engine.set_property("styleButtonColor"      .into(), QString::from("#fbfbfb").into());
                    engine.set_property("styleTextColor"        .into(), QString::from("#111111").into());
                    engine.set_property("styleAccentColor"      .into(), QString::from("#116cad").into());
                    engine.set_property("styleVideoBorderColor" .into(), QString::from("#d5d5d5").into());
                    engine.set_property("styleTextColorOnAccent".into(), QString::from("#ffffff").into());
                    engine.set_property("styleHrColor"          .into(), QString::from("#e5e5e5").into());
                    engine.set_property("stylePopupBorder"      .into(), QString::from("#d5d5d5").into());
                    engine.set_property("styleSliderHandle"     .into(), QString::from("#c2c2c2").into());
                    engine.set_property("styleSliderBackground" .into(), QString::from("#cdcdcd").into());
                    engine.set_property("styleHighlightColor"   .into(), QString::from("#10000000").into());
                },
                _ => { }
            }
        }
    }

    pub fn set_scaling(&self, dpi_scale: f64) {
        if let Some(engine) = self.engine_ptr {
            let engine = unsafe { &mut *(engine) };
            let dpi = cpp!(unsafe[] -> f64 as "double" { return QGuiApplication::primaryScreen()->logicalDotsPerInch() / 96.0; }) * dpi_scale;
            engine.set_property("dpiScale".into(), QVariant::from(dpi).into());
        }
    }

    pub fn set_icon(&mut self, wnd: QJSValue) {
        let hwnd = cpp!(unsafe [wnd as "QJSValue"] -> isize as "int64_t" {
            auto obj = qobject_cast<QQuickWindow *>(wnd.toQObject());
            obj->setIcon(QIcon(":/resources/icon.png"));
            return int64_t(obj->winId());
        });
        if self.main_window_handle.is_none() {
            self.main_window_handle = Some(hwnd);
            
            #[cfg(target_os = "windows")]
            unsafe {
                let _ = CoInitializeEx(std::ptr::null_mut(), COINIT_MULTITHREADED);
                if let Ok(tb) = CoCreateInstance(&TaskbarList, None, CLSCTX_ALL) {
                    self.taskbar = Some(tb);
                }
            }
        }
    }

    pub fn set_progress(&self, progress: f64) {
        const MAX_PROGRESS: u64 = 100_000;
        
        #[cfg(target_os = "windows")]
        if let Some(hwnd) = self.main_window_handle {
            let progress = (progress.clamp(0.0, 1.0) * MAX_PROGRESS as f64) as u64;
            unsafe {
                if let Some(ref tb) = self.taskbar {
                    let _ = tb.SetProgressValue(HWND(hwnd), progress, MAX_PROGRESS);
                }
            }
        }
    }

    pub fn init_calibrator(&mut self) {
        //if self.calibrator_ctl.is_none() {
            self.calibrator_ctl = Some(RefCell::new(Controller::new()));

            let calib_ctl = self.calibrator_ctl.as_ref().unwrap();
            calib_ctl.borrow().init_calibrator();
            let calib_ctlpinned = unsafe { QObjectPinned::new(calib_ctl) };
    
            if let Some(engine) = self.engine_ptr {
                let engine = unsafe { &mut *(engine) };
                engine.set_object_property("calib_controller".into(), calib_ctlpinned);

                calib_ctl.borrow_mut().stabilizer.params.write().framebuffer_inverted = util::is_opengl();
            }
        //}
    }
}
