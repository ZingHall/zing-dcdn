mod ipc;
mod components;

use dioxus::prelude::*;
use crate::components::toast::{Toast, ToastContainer};

#[derive(PartialEq, Clone)]
enum Tab {
    Dashboard,
    BlobBrowser,
    Cache,
    Staking,
    Settings,
}

#[component]
pub fn App() -> Element {
    let mut tab = use_signal(|| Tab::Dashboard);
    let toasts = use_signal(|| Vec::<Toast>::new());

    use_context_provider(|| toasts);

    use crate::components::dashboard::Dashboard;
    use crate::components::blob::BlobBrowser;
    use crate::components::cache::Cache;
    use crate::components::staking::Staking;
    use crate::components::settings::Settings;

    rsx! {
        div { class: "app",
            style: "font-family: system-ui, sans-serif; padding: 16px; max-width: 900px; margin: 0 auto;",
            h1 { style: "font-size: 1.2rem; margin: 0 0 8px 0;", "zing-cdn" }
            nav { style: "display: flex; gap: 8px; margin-bottom: 16px; border-bottom: 1px solid #ccc; padding-bottom: 8px;",
                button {
                    onclick: move |_| tab.set(Tab::Dashboard),
                    style: if tab() == Tab::Dashboard { "font-weight: bold" } else { "" },
                    "Dashboard"
                }
                button {
                    onclick: move |_| tab.set(Tab::BlobBrowser),
                    style: if tab() == Tab::BlobBrowser { "font-weight: bold" } else { "" },
                    "Blob Browser"
                }
                button {
                    onclick: move |_| tab.set(Tab::Cache),
                    style: if tab() == Tab::Cache { "font-weight: bold" } else { "" },
                    "Cache"
                }
                button {
                    onclick: move |_| tab.set(Tab::Staking),
                    style: if tab() == Tab::Staking { "font-weight: bold" } else { "" },
                    "Staking"
                }
                button {
                    onclick: move |_| tab.set(Tab::Settings),
                    style: if tab() == Tab::Settings { "font-weight: bold" } else { "" },
                    "⚙"
                }
            }
            div { class: "content",
                match tab() {
                    Tab::Dashboard => rsx! { Dashboard {} },
                    Tab::BlobBrowser => rsx! { BlobBrowser {} },
                    Tab::Cache => rsx! { Cache {} },
                    Tab::Staking => rsx! { Staking {} },
                    Tab::Settings => rsx! { Settings {} },
                }
            }
            ToastContainer {}
        }
    }
}
