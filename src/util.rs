// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use cpp::*;
use qmetaobject::*;

pub fn serde_json_to_qt_array(v: &serde_json::Value) -> QJsonArray {
    let mut ret = QJsonArray::default();
    if let Some(arr) = v.as_array() {
        for param in arr {
            match param {
                serde_json::Value::Number(v) => { ret.push(QJsonValue::from(v.as_f64().unwrap())); },
                serde_json::Value::Bool(v) => { ret.push(QJsonValue::from(*v)); },
                serde_json::Value::String(v) => { ret.push(QJsonValue::from(QString::from(v.clone()))); },
                serde_json::Value::Array(v) => { ret.push(QJsonValue::from(serde_json_to_qt_array(&serde_json::Value::Array(v.to_vec())))); },
                serde_json::Value::Object(_) => { ret.push(QJsonValue::from(serde_json_to_qt_object(param))); },
                serde_json::Value::Null => { /* ::log::warn!("null unimplemented");*/ }
            };
        }
    }
    ret
}
pub fn serde_json_to_qt_object(v: &serde_json::Value) -> QJsonObject {
    let mut map = QJsonObject::default();
    if let Some(obj) = v.as_object() {
        for (k, v) in obj {
            match v {
                serde_json::Value::Number(v) => { map.insert(k, QJsonValue::from(v.as_f64().unwrap())); },
                serde_json::Value::Bool(v) => { map.insert(k, QJsonValue::from(*v)); },
                serde_json::Value::String(v) => { map.insert(k, QJsonValue::from(QString::from(v.clone()))); },
                serde_json::Value::Array(v) => { map.insert(k, QJsonValue::from(serde_json_to_qt_array(&serde_json::Value::Array(v.to_vec())))); },
                serde_json::Value::Object(_) => { map.insert(k, QJsonValue::from(serde_json_to_qt_object(v))); },
                serde_json::Value::Null => { /* ::log::warn!("null unimplemented");*/ }
            };
        }
    }
    map
}

pub fn is_opengl() -> bool {
    cpp!(unsafe [] -> bool as "bool" {
        return QQuickWindow::graphicsApi() == QSGRendererInterface::OpenGLRhi;
    })
}

pub fn path_to_url(path: QString) -> QUrl {
    cpp!(unsafe [path as "QString"] -> QUrl as "QUrl" {
        return QUrl::fromLocalFile(path);
    })
}
pub fn url_to_path(url: QUrl) -> String {
    let path = cpp!(unsafe [url as "QUrl"] -> QString as "QString" {
        return url.toLocalFile();
    });
    path.to_string()
}

pub fn qt_queued_callback<T: QObject + 'static, T2: Send, F: FnMut(&T, T2) + 'static>(qobj: &T, mut cb: F) -> impl Fn(T2) + Send + Sync + Clone {
    let qptr = QPointer::from(qobj);
    qmetaobject::queued_callback(move |arg| {
        if let Some(this) = qptr.as_pinned() {
            let this = this.borrow();
            cb(this, arg);
        }
    })
}
pub fn qt_queued_callback_mut<T: QObject + 'static, T2: Send, F: FnMut(&mut T, T2) + 'static>(qobj: &T, mut cb: F) -> impl Fn(T2) + Send + Sync + Clone {
    let qptr = QPointer::from(qobj);
    qmetaobject::queued_callback(move |arg| {
        if let Some(this) = qptr.as_pinned() {
            let mut this = this.borrow_mut();
            cb(&mut this, arg);
        }
    })
}

#[macro_export]
macro_rules! wrap_simple_method {
    ($name:ident, $($param:ident:$type:ty),*) => {
        fn $name(&self, $($param:$type,)*) {
            self.stabilizer.$name($($param,)*);
        }
    };
    ($name:ident, $($param:ident:$type:ty),*; recompute) => {
        fn $name(&self, $($param:$type,)*) {
            self.stabilizer.$name($($param,)*);
            self.request_recompute();
        }
    };
    ($name:ident, $($param:ident:$type:ty),*; recompute$(; $extra_call:ident)*) => {
        fn $name(&mut self, $($param:$type,)*) {
            self.stabilizer.$name($($param,)*);
            self.request_recompute();
            $( self.$extra_call(); )*
        }
    };
}

cpp! {{
    #ifdef Q_OS_ANDROID
    #   include <QJniObject>
    #endif
    #include <QDesktopServices>
    #include <QStandardPaths>
    #include <QBuffer>
    #include <QImage>
    #include <QSettings>
    #include <QGuiApplication>
    #include <QObject>
    #include <QClipboard>
    #include <QEvent>

    class QtEventFilter : public QObject {
    public:
        QtEventFilter(std::function<void(QUrl)> cb) : m_cb(cb) { }
        bool eventFilter(QObject *obj, QEvent *event) override {
            if (event->type() == QEvent::FileOpen) {
                m_cb(static_cast<QFileOpenEvent *>(event)->url());
            }
            return QObject::eventFilter(obj, event);
        }
        std::function<void(QUrl)> m_cb;
    };
}}

pub fn resolve_android_url(url: QString) -> QString {
    cpp!(unsafe [url as "QString"] -> QString as "QString" {
        #ifdef Q_OS_ANDROID
            QVariant res = QNativeInterface::QAndroidApplication::runOnAndroidMainThread([url] {
                QJniObject jniPath = QJniObject::fromString(url);
                QJniObject jniUri = QJniObject::callStaticObjectMethod("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", jniPath.object());

                QJniObject activity(QNativeInterface::QAndroidApplication::context());

                QString url = QJniObject::callStaticObjectMethod("org/ekkescorner/utils/QSharePathResolver",
                    "getRealPathFromURI",
                    "(Landroid/content/Context;Landroid/net/Uri;)Ljava/lang/String;",
                    activity.object(), jniUri.object()
                ).toString();

                return QVariant::fromValue(url);
            }).result();
            return res.toString();
        #else
            return url;
        #endif
    })
}
pub fn catch_qt_file_open<F: FnMut(QUrl)>(cb: F) {
    let func: Box<dyn FnMut(QUrl)> = Box::new(cb);
    let cb_ptr = Box::into_raw(func);
    cpp!(unsafe [cb_ptr as "TraitObject2"] {
        qGuiApp->installEventFilter(new QtEventFilter([cb_ptr](QUrl url) {
            rust!(Rust_catch_qt_file_open [cb_ptr: *mut dyn FnMut(QUrl) as "TraitObject2", url: QUrl as "QUrl"] {
                let mut cb = unsafe { Box::from_raw(cb_ptr) };
                cb(url.clone());
                let _ = Box::into_raw(cb); // leak again so it doesn't get deleted here
            });
        }));
    });
}

pub fn open_file_externally(path: QString) {
    cpp!(unsafe [path as "QString"] { QDesktopServices::openUrl(QUrl::fromLocalFile(path)); });
}

pub fn get_data_location() -> String {
    cpp!(unsafe [] -> QString as "QString" {
        return QStandardPaths::writableLocation(QStandardPaths::AppDataLocation);
    }).into()
}

pub fn init_logging() {
    use simplelog::*;
    use std::path::*;
    let log_config = ConfigBuilder::new()
        .add_filter_ignore_str("mp4parse")
        .add_filter_ignore_str("wgpu")
        .add_filter_ignore_str("naga")
        .add_filter_ignore_str("akaze")
        .add_filter_ignore_str("ureq")
        .add_filter_ignore_str("rustls")
        .add_filter_ignore_str("mdk")
        .build();

    let file_log_config = ConfigBuilder::new()
        .add_filter_ignore_str("mp4parse")
        .add_filter_ignore_str("wgpu")
        .add_filter_ignore_str("naga")
        .add_filter_ignore_str("akaze")
        .add_filter_ignore_str("ureq")
        .add_filter_ignore_str("rustls")
        .build();

    #[cfg(target_os = "android")]
    WriteLogger::init(LevelFilter::Debug, log_config, crate::util::AndroidLog::default()).unwrap();

    #[cfg(not(target_os = "android"))]
    {
        let exe_loc = std::env::current_exe().map(|x| x.with_file_name("gyroflow.log")).unwrap_or_else(|_| PathBuf::from("./gyroflow.log"));
        if let Ok(file_log) = std::fs::File::create(exe_loc) {
            let _ = CombinedLogger::init(vec![
                TermLogger::new(LevelFilter::Debug, log_config, TerminalMode::Mixed, ColorChoice::Auto),
                WriteLogger::new(LevelFilter::Debug, file_log_config, file_log)
            ]);
        } else {
            let _ = TermLogger::init(LevelFilter::Debug, log_config, TerminalMode::Mixed, ColorChoice::Auto);
        }
    }

    qmetaobject::log::init_qt_to_rust();

    qml_video_rs::video_item::MDKVideoItem::setLogHandler(|level: i32, text: String| {
        match level {
            1 => { ::log::error!(target: "mdk", "[MDK] {}", text.trim()); },
            2 => { ::log::warn!(target: "mdk", "[MDK] {}", text.trim()); },
            3 => { ::log::info!(target: "mdk", "[MDK] {}", text.trim()); },
            4 => { ::log::debug!(target: "mdk", "[MDK] {}", text.trim()); },
            _ => { }
        }
    });
}

pub fn install_crash_handler() -> std::io::Result<()> {
    let cur_dir = std::env::current_dir()?;

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let os_str = cur_dir.as_os_str();
        let path: Vec<breakpad_sys::PathChar> = {
            #[cfg(windows)]
            {
                use std::os::windows::ffi::OsStrExt;
                os_str.encode_wide().collect()
            }
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStrExt;
                Vec::from(os_str.as_bytes())
            }
        };

        unsafe {
            extern "C" fn callback(path: *const breakpad_sys::PathChar, path_len: usize, _ctx: *mut std::ffi::c_void) {
                let path_slice = unsafe { std::slice::from_raw_parts(path, path_len) };

                let path = {
                    #[cfg(windows)]
                    {
                        use std::os::windows::ffi::OsStringExt;
                        std::path::PathBuf::from(std::ffi::OsString::from_wide(path_slice))
                    }
                    #[cfg(unix)]
                    {
                        use std::os::unix::ffi::OsStrExt;
                        std::path::PathBuf::from(std::ffi::OsStr::from_bytes(path_slice).to_owned())
                    }
                };

                println!("Crashdump written to {}", path.display());
            }

            breakpad_sys::attach_exception_handler(
                path.as_ptr(),
                path.len(),
                callback,
                std::ptr::null_mut(),
                breakpad_sys::INSTALL_BOTH_HANDLERS,
            );
        }
    }

    // Upload crash dumps
    crate::core::run_threaded(move || {
        if let Ok(files) = std::fs::read_dir(cur_dir) {
            for path in files.flatten() {
                let path = path.path();
                if path.to_string_lossy().ends_with(".dmp") {
                    if let Ok(content) = std::fs::read(&path) {
                        if let Ok(Ok(body)) = ureq::post("https://api.gyroflow.xyz/upload_dump").set("Content-Type", "application/octet-stream").send_bytes(&content).map(|x| x.into_string()) {
                            ::log::debug!("Minidump uploaded: {}", body.as_str());
                            let _ = std::fs::remove_file(path);
                        }
                    }
                }
            }
        }
    });
    Ok(())
}

#[cfg(target_os = "android")]
pub fn android_log(v: String) {
    use std::ffi::{CStr, CString};
    let tag = CStr::from_bytes_with_nul(b"Gyroflow\0").unwrap();
    if let Ok(msg) = CString::new(v) {
        unsafe {
            ndk_sys::__android_log_write(ndk_sys::android_LogPriority::ANDROID_LOG_DEBUG.0 as std::os::raw::c_int, tag.as_ptr(), msg.as_ptr());
        }
    }
}

#[cfg(target_os = "android")]
#[derive(Default)]
pub struct AndroidLog { buf: String }
#[cfg(target_os = "android")]
impl std::io::Write for AndroidLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Ok(s) = String::from_utf8(buf.to_vec()) {
            self.buf.push_str(&s);
        };
        if self.buf.contains('\n') {
            self.flush()?;
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { android_log(self.buf.clone()); self.buf.clear(); Ok(()) }
}

pub fn tr(context: &str, text: &str) -> String {
    let context = QString::from(context);
    let text = QString::from(text);
    cpp!(unsafe [context as "QString", text as "QString"] -> QString as "QString" {
        return QCoreApplication::translate(qUtf8Printable(context), qUtf8Printable(text));
    }).to_string()
}

pub fn qt_graphics_api() -> QString {
    cpp!(unsafe [] -> QString as "QString" {
        switch (QQuickWindow::graphicsApi()) {
            case QSGRendererInterface::OpenGL:     return "opengl";
            case QSGRendererInterface::Direct3D11: return "directx";
            case QSGRendererInterface::Vulkan:     return "vulkan";
            case QSGRendererInterface::Metal:      return "metal";
            default: return "unknown";
        }
    })
}

pub fn get_version() -> String {
    let ver = env!("CARGO_PKG_VERSION");
    if option_env!("GITHUB_REF").map_or(false, |x| x.contains("tags")) {
        ver.to_string() // Official, tagged version
    } else if let Some(gh_run) = option_env!("GITHUB_RUN_NUMBER") {
        format!("{} (gh{})", ver, gh_run)
    } else if let Some(time) = option_env!("BUILD_TIME") {
        format!("{} (dev{})", ver, time)
    } else {
        ver.to_string()
    }
}
pub fn clear_settings() {
    cpp!(unsafe [] { QSettings().clear(); })
}
pub fn get_setting(key: &str) -> String {
    let key = QString::from(key);
    cpp!(unsafe [key as "QString"] -> QString as "QString" { return QSettings().value(key).toString(); }).to_string()
}
pub fn set_setting(key: &str, value: &str) {
    let key = QString::from(key);
    let value = QString::from(value);
    cpp!(unsafe [key as "QString", value as "QString"] { QSettings().setValue(key, value); });
}
pub fn copy_to_clipboard(text: QString) {
    cpp!(unsafe [text as "QString"] { QGuiApplication::clipboard()->setText(text); })
}

pub fn save_exe_location() {
    if let Ok(exe_path) = std::env::current_exe() {
        if cfg!(target_os = "macos") {
            if let Some(parent) = exe_path.parent() { // MacOS
                if let Some(parent) = parent.parent() { // Contents
                    if let Some(parent) = parent.parent() { // Gyroflow.app
                        set_setting("exeLocation", &parent.to_string_lossy().to_string());
                    }
                }
            }
        } else if cfg!(target_os = "linux") {
            // TODO: AppImage
            set_setting("exeLocation", &exe_path.to_string_lossy().to_string());
        } else {
            set_setting("exeLocation", &exe_path.to_string_lossy().to_string());
        }
    }
}

pub fn image_data_to_base64(w: u32, h: u32, s: u32, data: &[u8]) -> QString {
    let ptr = data.as_ptr();
    cpp!(unsafe [w as "uint32_t", h as "uint32_t", s as "uint32_t", ptr as "const uint8_t *"] -> QString as "QString" {
        QImage img(ptr, w, h, s, QImage::Format_RGBA8888_Premultiplied);
        QByteArray byteArray;
        QBuffer buffer(&byteArray);
        buffer.open(QIODevice::WriteOnly);
        img.save(&buffer, "JPEG", 50);
        QString b64("data:image/jpg;base64,");
        b64.append(QString::fromLatin1(byteArray.toBase64().data()));
        return b64;
    })
}

pub fn image_to_b64(img: QImage) -> QString {
    cpp!(unsafe [img as "QImage"] -> QString as "QString" {
        QByteArray byteArray;
        QBuffer buffer(&byteArray);
        buffer.open(QIODevice::WriteOnly);
        img.save(&buffer, "JPEG", 50);
        QString b64("data:image/jpg;base64,");
        b64.append(QString::fromLatin1(byteArray.toBase64().data()));
        return b64;
    })
}
