
use qmetaobject::*;

#[derive(Default, QObject)]
pub struct Theme { 
    base: qt_base_class!(trait QObject), 
    set_theme: qt_method!(fn(theme: String)),

    pub engine_ptr: Option<*mut QmlEngine>
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
