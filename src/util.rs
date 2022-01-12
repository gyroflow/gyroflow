// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use cpp::*;
use qmetaobject::*;
use std::collections::HashMap;

pub fn serde_json_to_qt(v: &serde_json::Value) -> QJsonArray {
    let mut ret = QJsonArray::default();
    if let Some(arr) = v.as_array() {
        for param in arr {
            match param {
                serde_json::Value::Number(v) => { ret.push(QJsonValue::from(v.as_f64().unwrap())); },
                serde_json::Value::Bool(v) => { ret.push(QJsonValue::from(*v)); },
                serde_json::Value::String(v) => { ret.push(QJsonValue::from(QString::from(v.clone()))); },
                serde_json::Value::Array(v) => { ret.push(QJsonValue::from(serde_json_to_qt(&serde_json::Value::Array(v.to_vec())))); },
                serde_json::Value::Object(obj) => {
                    let mut map = QJsonObject::default();
                    for (k, v) in obj {
                        match v {
                            serde_json::Value::Number(v) => { map.insert(k, QJsonValue::from(v.as_f64().unwrap())); },
                            serde_json::Value::Bool(v) => { map.insert(k, QJsonValue::from(*v)); },
                            serde_json::Value::String(v) => { map.insert(k, QJsonValue::from(QString::from(v.clone()))); },
                            serde_json::Value::Array(v) => { map.insert(k, QJsonValue::from(serde_json_to_qt(&serde_json::Value::Array(v.to_vec())))); },
                            _ => { ::log::warn!("Unimplemented"); }
                        };
                    }
                    ret.push(QJsonValue::from(map));
                },
                _ => { ::log::warn!("Unimplemented"); }
            };
        }
    }
    ret
}
pub fn qjsonobject_to_hashmap(o: &QJsonObject) -> HashMap<String, String> {
    let mut ret = HashMap::new();
    let keys = o.keys();
    for k in keys {
        let val: QString = o.value(&k).into();
        ret.insert(k, val.to_string());
    }
    ret
}

pub fn is_opengl() -> bool {
    cpp!(unsafe [] -> bool as "bool" {
        return QQuickWindow::graphicsApi() == QSGRendererInterface::OpenGLRhi;
    })
}

pub fn url_to_path(url: QUrl) -> String {
    let path = cpp!(unsafe [url as "QUrl"] -> QString as "QString" {
        return url.toLocalFile();
    });
    path.to_string()    
}

pub fn qt_queued_callback<T: QObject + 'static, T2: Send, F: FnMut(&T, T2) + 'static>(qobj: &T, mut cb: F) -> impl Fn(T2) + Send + Sync + Clone {
    let qptr = QPointer::from(&*qobj);
    qmetaobject::queued_callback(move |arg| {
        if let Some(this) = qptr.as_pinned() {
            let this = this.borrow();
            cb(this, arg);
        }
    })
}
pub fn qt_queued_callback_mut<T: QObject + 'static, T2: Send, F: FnMut(&mut T, T2) + 'static>(qobj: &T, mut cb: F) -> impl Fn(T2) + Send + Sync + Clone {
    let qptr = QPointer::from(&*qobj);
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
    ($name:ident, $($param:ident:$type:ty),*; recompute; $extra_call:ident) => {
        fn $name(&mut self, $($param:$type,)*) {
            self.stabilizer.$name($($param,)*);
            self.request_recompute();
            self.$extra_call();
        }
    };
}

cpp! {{
    #ifdef Q_OS_ANDROID
    #   include <QJniObject>
    #endif
    #include <QDesktopServices>
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

pub fn open_file_externally(path: QString) {
    cpp!(unsafe [path as "QString"] { QDesktopServices::openUrl(QUrl::fromLocalFile(path)); });
}

#[cfg(target_os = "android")]
pub fn android_log(v: String) {
    use std::ffi::{CStr, CString};
    let tag = CStr::from_bytes_with_nul(b"Gyroflow\0").unwrap();
    if let Ok(msg) = CString::new(v) {
        unsafe {
            ndk_sys::__android_log_write(ndk_sys::android_LogPriority_ANDROID_LOG_DEBUG as std::os::raw::c_int, tag.as_ptr(), msg.as_ptr());
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
