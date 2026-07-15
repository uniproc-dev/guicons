//! cargo run --example iced_icon --features iced

use iced::widget::{container, svg};
use iced::Element;

fn main() -> iced::Result {
    iced::run(|_state: &mut (), _message: ()| {}, view)
}

fn view(_state: &()) -> Element<'_, ()> {
    let handle = guicons::icon!("mdi:home");
    container(svg(handle).width(64).height(64)).into()
}
