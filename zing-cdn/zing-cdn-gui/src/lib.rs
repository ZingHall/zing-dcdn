mod ipc;
mod components;

pub use components::*;

use dioxus::prelude::*;

#[derive(PartialEq, Clone)]
enum Tab {
    Dashboard,
    BlobBrowser,
    Cache,
}

#[component]
pub fn App() -> Element {
    let tab = use_signal(|| Tab::Dashboard);

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
            }
            div { class: "content",
                match tab() {
                    Tab::Dashboard => rsx! { Dashboard {} },
                    Tab::BlobBrowser => rsx! { BlobBrowser {} },
                    Tab::Cache => rsx! { Cache {} },
                }
            }
        }
    }
}
