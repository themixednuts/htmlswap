mod base {
    include!(concat!(env!("OUT_DIR"), "/base_view.rs"));

    impl BaseView {
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
}

mod components {
    include!(concat!(env!("OUT_DIR"), "/components_view.rs"));

    #[derive(Clone)]
    struct SmokeItem {
        label: &'static str,
    }

    impl SmokeItem {
        fn on_select<E>(
            &self,
            _event: &E,
            _window: &mut gpui::Window,
            _cx: &mut gpui::Context<ComponentsView>,
        ) {
        }
    }

    impl ComponentsView {
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
}

fn main() {
    // Starting an app on gpui-pre needs its platform crate (`gpui_platform`
    // or `gpui_kit::application()`), which generated views do not; naming
    // both views type-checks everything they emit.
    let _ = (
        components::ComponentsView::new,
        base::BaseView::new,
    );
}
