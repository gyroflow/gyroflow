// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Adrian <adrian.eddy at gmail>

#![allow(non_snake_case)]

use qmetaobject::*;
use cpp::cpp;

#[derive(Default, QObject)]
pub struct Settings {
    base: qt_base_class!(trait QObject),

    init: qt_method!(fn(&mut self, obj: QJSValue)),
    value: qt_method!(fn(&self, key: QString, default: QVariant) -> QVariant),
    contains: qt_method!(fn(&self, key: QString) -> bool),
    setValue: qt_method!(fn(&mut self, key: QString, value: QVariant)),
    clear: qt_method!(fn(&mut self)),
    dataDir: qt_method!(fn(&self, path: QString) -> QString),

    propChanged: qt_method!(fn(&self, obj: QJSValue)),
}

impl Settings {
    fn value(&self, key: QString, default: QVariant) -> QVariant {
        let key = key.to_string();
        serde_json_to_qvariant(gyroflow_core::settings::get(&key, qvariant_to_serde_json(default)))
    }
    fn contains(&self, key: QString) -> bool {
        let key = key.to_string();
        gyroflow_core::settings::contains(&key)
    }
    fn setValue(&self, key: QString, val: QVariant) {
        let key = key.to_string();
        gyroflow_core::settings::set(&key, qvariant_to_serde_json(val))
    }
    fn clear(&self) {
        gyroflow_core::settings::clear()
    }
    fn dataDir(&self, path: QString) -> QString {
        let path = path.to_string();
        QString::from(gyroflow_core::settings::data_dir().join(path).to_string_lossy().to_string())
    }
    ///////////////////////////////////////////////////////////////////

    fn propChanged(&self, v: QJSValue) {
        let props = cpp!(unsafe [v as "QJSValue"] -> QVariantMap as "QVariantMap" {
            QObject *obj = v.toQObject();
            if (!obj) {
                qDebug() << "settings.propChanged(): null QObject!";
                return QVariantMap();
            }
            const QMetaObject *mo = obj->metaObject();
            const int offset = mo->propertyOffset();
            const int count = mo->propertyCount();

            QVariantMap changedProperties;

            for (int i = offset; i < count; ++i) {
                const QMetaProperty &property = mo->property(i);
                QVariant value = property.read(obj);
                if (value.metaType() == QMetaType::fromType<QJSValue>()) value = value.value<QJSValue>().toVariant();
                changedProperties.insert(property.name(), value);
            }

            return changedProperties;
        });
        for (k, v) in &props {
            self.setValue(k.clone(), v.clone());
        }
    }
    fn init(&mut self, v: QJSValue) {
        let sett = self.get_cpp_object();
        cpp!(unsafe [sett as "QObject *", v as "QJSValue"] {
            QObject *obj = v.toQObject();
            if (!obj) {
                qDebug() << "settings.init(): null QObject!";
                return;
            }
            const QMetaObject *mo = obj->metaObject();
            const int offset = mo->propertyOffset();
            const int count = mo->propertyCount();

            for (int i = offset; i < count; ++i) {
                QMetaProperty property = mo->property(i);
                const QString propertyName = QString::fromUtf8(property.name());

                QVariant previousValue = property.read(obj);
                if (previousValue.metaType() == QMetaType::fromType<QJSValue>()) previousValue = previousValue.value<QJSValue>().toVariant();

                QVariant currentValue;
                QMetaObject::invokeMethod(sett, "value", Q_RETURN_ARG(QVariant, currentValue), Q_ARG(QString, propertyName), Q_ARG(QVariant, previousValue));

                if (!currentValue.isNull() && (!previousValue.isValid() || (currentValue.canConvert(previousValue.metaType()) && previousValue != currentValue))) {
                    property.write(obj, currentValue);
                }

                // ensure that a non-existent setting gets written even if the property wouldn't change later
                bool exists = false;
                QMetaObject::invokeMethod(sett, "contains", Q_RETURN_ARG(bool, exists), Q_ARG(QString, propertyName));
                if (!exists)
                    QMetaObject::invokeMethod(obj, "propChanged", Qt::QueuedConnection);

                // setup change notifications
                if (property.hasNotifySignal()) {
                    QMetaObject::connect(obj, property.notifySignalIndex(), obj, mo->indexOfSlot("propChanged()"), Qt::QueuedConnection);
                }
            }
        });
    }
}

fn serde_json_to_qvariant(v: serde_json::Value) -> QVariant {
    match v {
        serde_json::Value::Number(v) => { v.as_f64().unwrap().into() },
        serde_json::Value::Bool(v)   => { v.into() },
        serde_json::Value::String(v) => { QString::from(v.clone()).into() },
        serde_json::Value::Array(v)  => { ::log::error!("Array {v:?}"); QVariant::default() },
        serde_json::Value::Object(v) => { ::log::error!("Object {v:?}"); QVariant::default() },
        serde_json::Value::Null      => { QVariant::default() }
    }
}

fn qvariant_to_serde_json(v: QVariant) -> serde_json::Value {
    use serde_json::{ Value, Number };
    let v = &v;
    match v.user_type() {
        0  => Value::Null,
        1  => Value::Bool(v.to_bool()),
        2  => Value::Number(cpp!(unsafe [v as "const QVariant*"] -> i32 as "int"                { return v->toInt();       }).into()),
        3  => Value::Number(cpp!(unsafe [v as "const QVariant*"] -> u32 as "uint"               { return v->toUInt();      }).into()),
        4  => Value::Number(cpp!(unsafe [v as "const QVariant*"] -> i64 as "long long"          { return v->toLongLong();  }).into()),
        5  => Value::Number(cpp!(unsafe [v as "const QVariant*"] -> u64 as "unsigned long long" { return v->toULongLong(); }).into()),
        10 => Value::String(v.to_qstring().to_string()),
        6  => { // QMetaType::Double
            let num = Number::from_f64(cpp!(unsafe [v as "const QVariant*"] -> f64 as "double" { return v->toDouble(); }));
            if let Some(num) = num {
                Value::Number(num)
            } else {
                Value::Null
            }
        },
        38 => { // QMetaType::Float
            let num = Number::from_f64(cpp!(unsafe [v as "const QVariant*"] -> f32 as "float" { return v->toFloat(); }) as f64);
            if let Some(num) = num {
                Value::Number(num)
            } else {
                Value::Null
            }
        },
        32 => Value::Number(cpp!(unsafe [v as "const QVariant*"] -> i32 as "int"            { return v->toInt();  }).into()), // long
        33 => Value::Number(cpp!(unsafe [v as "const QVariant*"] -> i16 as "short"          { return v->toInt();  }).into()),
        35 => Value::Number(cpp!(unsafe [v as "const QVariant*"] -> u32 as "unsigned int"   { return v->toUInt(); }).into()), // unsigned long
        36 => Value::Number(cpp!(unsafe [v as "const QVariant*"] -> u16 as "unsigned short" { return v->toUInt(); }).into()),
        43 => Value::Null, // QMetaType::Void
        // 7  | QMetaType::QChar      | QChar
        // 12 | QMetaType::QByteArray | QByteArray
        // 51 | QMetaType::Nullptr    | std::nullptr_t
        // 31 | QMetaType::VoidStar   | void *
        // 34 | QMetaType::Char       | char
        // 56 | QMetaType::Char16     | char16_t
        // 57 | QMetaType::Char32     | char32_t
        // 40 | QMetaType::SChar      | signed char
        // 37 | QMetaType::UChar      | unsigned char
        // 41 | QMetaType::QVariant   | QVariant
        _ => {
            ::log::error!("Unknown QVariant type: {}", v.user_type());
            Value::Null
        }
    }
}
