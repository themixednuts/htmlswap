include!(concat!(env!("OUT_DIR"), "/base_smoke_view.rs"));

impl BaseSmokeView {
    fn name(&self) -> &'static str {
        "Aether"
    }

    fn on_name(
        &mut self,
        _value: &str,
        _window: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) {
    }
}

fn main() {
    use gpui::{App, Application, Bounds, WindowBounds, WindowOptions, px, size};

    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(640.), px(360.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| BaseSmokeView::new(window, cx)),
        )
        .expect("smoke window should open");
        cx.activate(true);
    });
}
