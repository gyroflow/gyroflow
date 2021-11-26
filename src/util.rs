use cpp::*;
use qmetaobject::*;

pub fn simd_json_to_qt(v: &simd_json::owned::Value) -> QJsonArray {
    let mut arr = QJsonArray::default();
    use simd_json::ValueAccess;
    for param in v.as_array().unwrap() {
        let mut map = QJsonObject::default();
        for (k, v) in param.as_object().unwrap() {
            match v {
                simd_json::OwnedValue::Static(simd_json::StaticNode::F64(v)) => { map.insert(k, QJsonValue::from(*v)); },
                simd_json::OwnedValue::Static(simd_json::StaticNode::I64(v)) => { map.insert(k, QJsonValue::from(*v as f64)); },
                simd_json::OwnedValue::Static(simd_json::StaticNode::U64(v)) => { map.insert(k, QJsonValue::from(*v as f64)); },
                simd_json::OwnedValue::Static(simd_json::StaticNode::Bool(v)) => { map.insert(k, QJsonValue::from(*v)); },
                simd_json::OwnedValue::String(v) => { map.insert(k, QJsonValue::from(QString::from(v.clone()))); },
                _ => { println!("Unimplemented"); }
            };
        }
        arr.push(QJsonValue::from(map));
    }
    arr
}

pub fn url_to_path(url: &str) -> &str {
    if url.starts_with("file://") {
        if cfg!(target_os = "windows") {
            url.strip_prefix("file:///").unwrap()
        } else {
            url.strip_prefix("file://").unwrap()
        }
    } else {
        url
    }
}

cpp! {{
    #ifdef Q_OS_ANDROID
    #   include <QJniObject>
    #endif
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
