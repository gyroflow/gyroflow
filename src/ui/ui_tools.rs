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
    #include <QJsonObject>
    #include <qpa/qplatformwindow.h>
}}

#[derive(Default, QObject)]
pub struct UITools {
    base: qt_base_class!(trait QObject),
    set_theme: qt_method!(fn(&mut self, theme: String)),
    set_language: qt_method!(fn(&self, lang_id: QString)),
    get_default_language: qt_method!(fn(&self) -> QString),
    set_scaling: qt_method!(fn(&self, dpiScale: f64)),
    init_calibrator: qt_method!(fn(&mut self)),
    set_icon: qt_method!(fn(&mut self, wnd: QJSValue)),
    get_safe_area_margins: qt_method!(fn(&mut self, wnd: QJSValue) -> QJsonObject),
    set_progress: qt_method!(fn(&self, progress: f64)),
    modify_digit: qt_method!(fn(&self, value: String, cursor_position: usize, increase: bool) -> QString),
    closing: qt_method!(fn(&mut self)),

    language_changed: qt_signal!(),

    calibrator_ctl: Option<RefCell<Controller>>,

    #[cfg(target_os = "windows")]
    taskbar: Option<ITaskbarList4>,

    main_window_handle: Option<isize>,

    is_dark: bool,

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
            if (lang2 == "zh") {
                // If Chinese but unknown locale, use Chinese Simplified by default
                return "zh_CN";
            }
            return "en";
        })
    }

    pub fn set_theme(&mut self, theme: String) {
        if let Some(engine) = self.engine_ptr {
            let engine = unsafe { &mut *(engine) };

            cpp!(unsafe [] { auto f = QGuiApplication::font(); f.setFamily("Arial"); QGuiApplication::setFont(f); });
            engine.set_property("styleFont".into(), QString::from("Arial").into());

            let force_mobile = theme.starts_with("mobile_");
            let force_destop = theme.starts_with("desktop_");
            engine.set_property("forceMobileLayout".into(), force_mobile.into());
            engine.set_property("forceDesktopLayout".into(), force_destop.into());

            self.is_dark = theme.contains("dark");

            match self.is_dark {
                true => {
                    engine.set_property("style"                 .into(), QString::from("dark").into());
                    engine.set_property("styleBackground"       .into(), QString::from("#1e1e1e").into());
                    engine.set_property("styleBackground2"      .into(), QString::from("#191919").into());
                    engine.set_property("styleButtonColor"      .into(), QString::from("#282828").into());
                    engine.set_property("styleTextColor"        .into(), QString::from("#ffffff").into());
                    engine.set_property("styleAccentColor"      .into(), QString::from("#76baed").into());
                    engine.set_property("styleVideoBorderColor" .into(), QString::from("#2b2b2b").into());
                    engine.set_property("styleTextColorOnAccent".into(), QString::from("#000000").into());
                    engine.set_property("styleHrColor"          .into(), QString::from("#2e2e2e").into());
                    engine.set_property("stylePopupBorder"      .into(), QString::from("#0f0f0f").into());
                    engine.set_property("styleSliderHandle"     .into(), QString::from("#454545").into());
                    engine.set_property("styleSliderBackground" .into(), QString::from("#949494").into());
                    engine.set_property("styleHighlightColor"   .into(), QString::from("#10ffffff").into());
                },
                false => {
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
                }
            }
            self.update_dark_mode(0);
        }
    }

    pub fn set_scaling(&self, dpi_scale: f64) {
        if let Some(engine) = self.engine_ptr {
            let engine = unsafe { &mut *(engine) };
            let mut dpi = cpp!(unsafe[] -> f64 as "double" { return QGuiApplication::primaryScreen()->logicalDotsPerInch() / 96.0; }) * dpi_scale;
            if cfg!(any(target_os = "android", target_os = "ios")) {
                dpi *= 1.2;
            }
            engine.set_property("dpiScale".into(), QVariant::from(dpi));
        }
    }

    pub fn get_safe_area_margins(&mut self, wnd: QJSValue) -> QJsonObject {
        cpp!(unsafe [wnd as "QJSValue"] -> QJsonObject as "QJsonObject" {
            auto obj = qobject_cast<QQuickWindow *>(wnd.toQObject());
            QPlatformWindow *pWin = qobject_cast<QWindow *>(obj)->handle();
            QMargins safeArea = pWin->safeAreaMargins();
            return QJsonObject {
                { "top",    safeArea.top() },
                { "bottom", safeArea.bottom() },
                { "right",  safeArea.right() },
                { "left",   safeArea.left() }
            };
        })
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
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                if let Ok(tb) = CoCreateInstance(&TaskbarList, None, CLSCTX_ALL) {
                    self.taskbar = Some(tb);
                }
            }
        }
        self.update_dark_mode(hwnd);
    }

    #[allow(unused_mut, unused_variables)]
    fn update_dark_mode(&self, mut hwnd: isize) {
        #[cfg(target_os = "windows")]
        unsafe {
            if hwnd == 0 && self.main_window_handle.is_some() { hwnd = self.main_window_handle.unwrap(); }
            use windows::Win32::Foundation::*;
            use windows::Win32::Graphics::Dwm::*;
            let is_dark = BOOL::from(self.is_dark);
            let _ = DwmSetWindowAttribute(HWND(hwnd), DWMWA_USE_IMMERSIVE_DARK_MODE, &is_dark as *const _ as _, std::mem::size_of_val(&is_dark) as _);
        }
    }

    pub fn set_progress(&self, _progress: f64) {
        #[cfg(target_os = "windows")]
        if let Some(hwnd) = self.main_window_handle {
            const MAX_PROGRESS: u64 = 100_000;
            let progress = (_progress.clamp(0.0, 1.0) * MAX_PROGRESS as f64) as u64;
            unsafe {
                if let Some(ref tb) = self.taskbar {
                    let _ = tb.SetProgressValue(HWND(hwnd), progress, MAX_PROGRESS);
                }
            }
        }
    }

    pub fn closing(&mut self) {
        #[cfg(target_os = "windows")]
        {
            self.taskbar = None;
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

    pub fn modify_digit(&self, value: String, cursor_position: usize, increase: bool) -> QString {
        let (new_num_str, new_cursor_position) = modify_digit_impl(value.as_str(), cursor_position, increase);

        QString::from(format!("{};{}", new_num_str, new_cursor_position))
    }
}


pub fn modify_digit_impl(num_str: &str, cursor_position: usize, increase: bool) -> (String, usize) {
    // Convert the string to a number and find its decimal point
    let number = num_str.parse::<f64>().unwrap();
    let number_length = num_str.len();
    let decimal_pos = num_str.find('.').unwrap_or(number_length);
    let has_decimal_part = number_length > decimal_pos;
    let decimal_length = if has_decimal_part { number_length - decimal_pos - 1 } else { 0 };

    assert!(cursor_position <= number_length, "Cursor position out of bounds");

    // Calculate the position offset from the decimal point
    let mut pos_adjust: isize = 0;
    if cursor_position > decimal_pos {
        pos_adjust -= 1;
    } else if has_decimal_part && cursor_position == decimal_pos {
        pos_adjust -= 1;
    }
    if cursor_position == number_length {
        pos_adjust -= 1;
    } else if cursor_position == 0 && number < 0.0 {
        pos_adjust += 1;
    }

    // Calculate the value to add or subtract
    let dec_offset: isize = (-1 * pos_adjust)
        .checked_add_unsigned(decimal_pos).unwrap()
        .checked_sub_unsigned(cursor_position + 1).unwrap();
    let modifier = 10f64.powi(dec_offset as i32);

    // Modify the number
    let new_number = if increase {
        if number < 0.0 && modifier + number > 0.0 { -1.0 * number } else { number + modifier }
    } else {
        if number > 0.0 && modifier - number > 0.0 { -1.0 * number } else { number - modifier }
    };
    let new_num_negative = new_number < 0.0;

    // Calculate padding length
    let new_num_str_abs = format!("{:1.1$}", new_number.abs(), decimal_length);
    let new_num_length = new_num_str_abs.len() + if new_num_negative { 1 } else { 0 };
    let new_num_length_diff: isize = if (number < 0.0) ^ (new_number < 0.0) {
        if number < 0.0 { 1 } else { -1 }
    } else {
        0
    };
    let new_num_pad_length = number_length.checked_sub(new_num_length.checked_add_signed(new_num_length_diff).unwrap()).unwrap_or(0);

    // Adjust the cursor position if the number of digits has changed
    let new_num_str = format!(
        "{}{}{}",
        if new_num_negative { "-" } else { "" },
        "0".repeat(new_num_pad_length),
        new_num_str_abs,
    );
    let new_cursor_position = (new_num_length + new_num_pad_length).saturating_sub(number_length - cursor_position);

    (new_num_str, new_cursor_position)
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;

    #[test_case(("0", 0, true), ("1", 0))]
    #[test_case(("0", 0, false), ("-1", 1))]
    #[test_case(("0", 1, true), ("1", 1))]
    #[test_case(("0", 1, false), ("-1", 2))]
    #[test_case(("00", 0, true), ("10", 0))]
    #[test_case(("00", 0, false), ("-10", 1))]
    #[test_case(("00", 1, true), ("01", 1))]
    #[test_case(("00", 1, false), ("-01", 2))]
    #[test_case(("1", 0, true), ("2", 0))]
    #[test_case(("1", 0, false), ("0", 0))]
    #[test_case(("1", 1, true), ("2", 1))]
    #[test_case(("1", 1, false), ("0", 1))]
    #[test_case(("-1", 0, true), ("0", 0))]
    #[test_case(("-1", 0, false), ("-2", 0))]
    #[test_case(("-1", 1, true), ("0", 0))]
    #[test_case(("-1", 1, false), ("-2", 1))]
    #[test_case(("-1", 2, true), ("0", 1))]
    #[test_case(("-1", 2, false), ("-2", 2))]
    #[test_case(("9", 0, true), ("10", 1))]
    #[test_case(("9", 0, false), ("8", 0))]
    #[test_case(("-9", 1, true), ("-8", 1))]
    #[test_case(("-9", 1, false), ("-10", 2))]
    #[test_case(("10", 0, true), ("20", 0))]
    #[test_case(("10", 0, false), ("00", 0))]
    #[test_case(("10", 1, true), ("11", 1))]
    #[test_case(("10", 1, false), ("09", 1))]
    #[test_case(("10", 2, true), ("11", 2))]
    #[test_case(("10", 2, false), ("09", 2))]
    #[test_case(("-10", 0, true), ("00", 0))]
    #[test_case(("-10", 0, false), ("-20", 0))]
    #[test_case(("-10", 1, true), ("00", 0))]
    #[test_case(("-10", 1, false), ("-20", 1))]
    #[test_case(("-10", 2, true), ("-09", 2))]
    #[test_case(("-10", 2, false), ("-11", 2))]
    #[test_case(("-10", 3, true), ("-09", 3))]
    #[test_case(("-10", 3, false), ("-11", 3))]
    #[test_case(("11", 0, true), ("21", 0))]
    #[test_case(("11", 0, false), ("01", 0))]
    #[test_case(("11", 1, true), ("12", 1))]
    #[test_case(("11", 1, false), ("10", 1))]
    #[test_case(("-11", 1, true), ("-01", 1))]
    #[test_case(("-11", 1, false), ("-21", 1))]
    #[test_case(("-11", 2, true), ("-10", 2))]
    #[test_case(("-11", 2, false), ("-12", 2))]
    #[test_case(("19", 0, true), ("29", 0))]
    #[test_case(("19", 0, false), ("09", 0))]
    #[test_case(("19", 1, true), ("20", 1))]
    #[test_case(("19", 1, false), ("18", 1))]
    #[test_case(("-19", 1, true), ("-09", 1))]
    #[test_case(("-19", 1, false), ("-29", 1))]
    #[test_case(("-19", 2, true), ("-18", 2))]
    #[test_case(("-19", 2, false), ("-20", 2))]
    #[test_case(("999", 2, true), ("1000", 3))]
    #[test_case(("999", 2, false), ("998", 2))]
    #[test_case(("-999", 3, true), ("-998", 3))]
    #[test_case(("-999", 3, false), ("-1000", 4))]
    #[test_case(("0.0", 0, true), ("1.0", 0))]
    #[test_case(("0.0", 0, false), ("-1.0", 1))]
    #[test_case(("0.0", 1, true), ("1.0", 1))]
    #[test_case(("0.0", 1, false), ("-1.0", 2))]
    #[test_case(("0.0", 2, true), ("0.1", 2))]
    #[test_case(("0.0", 2, false), ("-0.1", 3))]
    #[test_case(("0.0", 3, true), ("0.1", 3))]
    #[test_case(("0.0", 3, false), ("-0.1", 4))]
    #[test_case(("0.00", 2, true), ("0.10", 2))]
    #[test_case(("0.00", 2, false), ("-0.10", 3))]
    #[test_case(("0.00", 3, true), ("0.01", 3))]
    #[test_case(("0.00", 3, false), ("-0.01", 4))]
    #[test_case(("-0.2", 0, true), ("0.2", 0))]
    #[test_case(("-0.2", 0, false), ("-1.2", 0))]
    #[test_case(("0.2", 0, true), ("1.2", 0))]
    #[test_case(("0.2", 0, false), ("-0.2", 1))]
    #[test_case(("-0.5", 1, true), ("0.5", 0))]
    #[test_case(("0.5", 0, false), ("-0.5", 1))]
    #[test_case(("-00.5", 2, true), ("00.5", 1))]
    #[test_case(("00.5", 1, false), ("-00.5", 2))]
    #[test_case(("1.0", 0, true), ("2.0", 0))]
    #[test_case(("1.0", 0, false), ("0.0", 0))]
    #[test_case(("1.0", 2, true), ("1.1", 2))]
    #[test_case(("1.0", 2, false), ("0.9", 2))]
    #[test_case(("-1.0", 1, true), ("0.0", 0))]
    #[test_case(("-1.0", 1, false), ("-2.0", 1))]
    #[test_case(("-1.0", 3, true), ("-0.9", 3))]
    #[test_case(("-1.0", 3, false), ("-1.1", 3))]
    #[test_case(("12.345", 0, true), ("22.345", 0))]
    #[test_case(("12.345", 1, true), ("13.345", 1))]
    #[test_case(("12.345", 2, true), ("13.345", 2))]
    #[test_case(("12.345", 3, true), ("12.445", 3))]
    #[test_case(("12.345", 4, true), ("12.355", 4))]
    #[test_case(("12.345", 5, true), ("12.346", 5))]
    #[test_case(("12.345", 6, true), ("12.346", 6))]
    #[test_case(("12.345", 0, false), ("02.345", 0))]
    #[test_case(("12.345", 1, false), ("11.345", 1))]
    #[test_case(("12.345", 2, false), ("11.345", 2))]
    #[test_case(("12.345", 3, false), ("12.245", 3))]
    #[test_case(("12.345", 4, false), ("12.335", 4))]
    #[test_case(("12.345", 5, false), ("12.344", 5))]
    #[test_case(("12.345", 6, false), ("12.344", 6))]
    fn test_modify_digit_impl(input: (&str, usize, bool), expected: (&str, usize)) {
        let actual = modify_digit_impl(input.0, input.1, input.2);

        assert_eq!(
            expected,
            (actual.0.as_str(), actual.1),
            "test_case((\"{}\", {}, {}), (\"{}\", {}))\n input: {}{}  key: [{}]\nexpect: \"{}|{}\"\nactual: \"{}|{}\"",
            input.0,
            input.1,
            input.2,
            expected.0,
            expected.1,
            if expected.0.len() > input.0.len() { " " } else { "" },
            format!("\"{}{}{}\"", input.0.get(0..input.1).unwrap_or(""), "|", input.0.get(input.1..).unwrap_or("")),
            if input.2 { "↑" } else { "↓" },
            expected.0.get(0..expected.1).unwrap_or(""),
            expected.0.get(expected.1..).unwrap_or(""),
            actual.0.get(0..actual.1).unwrap_or(""),
            actual.0.get(actual.1..).unwrap_or(""),
        );
    }
}
