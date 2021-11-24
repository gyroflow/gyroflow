
use qmetaobject::*;
use cpp::*;

#[derive(Default, QObject)]
pub struct Theme { 
    base: qt_base_class!(trait QObject), 
    set_theme: qt_method!(fn(theme: String)),

    pub engine_ptr: Option<*mut QmlEngine>
}
impl Theme {
    pub fn set_theme(&self, theme: String) {
        let engine = unsafe { &mut *(self.engine_ptr.unwrap()) };

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
