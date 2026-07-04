include!(concat!(env!("OUT_DIR"), "/smoke_view.rs"));

#[derive(Clone)]
struct SmokeItem {
    label: &'static str,
}

impl SmokeItem {
    fn on_select<E>(
        &self,
        _event: &E,
        _window: &mut gpui::Window,
        _cx: &mut gpui::Context<SmokeView>,
    ) {
    }
}

impl SmokeView {
    fn items(&self) -> Vec<SmokeItem> {
        vec![
            SmokeItem { label: "Terrain" },
            SmokeItem { label: "Lighting" },
            SmokeItem { label: "Gameplay" },
        ]
    }

    fn save<E>(
        &mut self,
        _event: &E,
        _window: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) {
    }
}

fn main() {
    use gpui::{App, Application, Bounds, WindowBounds, WindowOptions, px, size};

    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(960.), px(640.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui_component::TitleBar::title_bar_options()),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| SmokeView::new(window, cx)),
        )
        .expect("smoke window should open");
        cx.activate(true);
    });
}
