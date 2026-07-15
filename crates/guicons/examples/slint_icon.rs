//! cargo run --example slint_icon --features slint

fn main() {
    slint::slint! {
        export component App inherits Window {
            in property <image> icon-image;
            VerticalLayout {
                alignment: center;
                Image {
                    source: icon-image;
                    width: 64px;
                    height: 64px;
                }
            }
        }
    }

    let app = App::new().unwrap();
    app.set_icon_image(guicons::icon!("mdi:home"));
    app.run().unwrap();
}
