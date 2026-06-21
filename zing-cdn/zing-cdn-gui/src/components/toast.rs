use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub enum ToastLevel {
    Error,
    Success,
    Info,
}

#[derive(Clone, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub message: String,
    pub level: ToastLevel,
}

static NEXT_ID: GlobalSignal<u64> = Signal::global(|| 0);

pub fn use_toasts() -> Signal<Vec<Toast>> {
    use_context()
}

pub fn add_toast(message: &str, level: ToastLevel) {
    let mut toasts = use_toasts();
    let id = *NEXT_ID.peek();
    NEXT_ID.with_mut(|n| *n = n.wrapping_add(1));
    toasts.push(Toast { id, message: message.to_string(), level });
}

pub fn remove_toast(id: u64) {
    let mut toasts = use_toasts();
    toasts.retain(|t| t.id != id);
}

#[component]
pub fn ToastContainer() -> Element {
    let toasts = use_toasts();

    rsx! {
        div {
            style: "position: fixed; bottom: 20px; right: 20px; display: flex; flex-direction: column; gap: 8px; z-index: 9999;",
            for toast in toasts.iter() {
                ToastItem { toast: toast.clone() }
            }
        }
    }
}

#[component]
fn ToastItem(toast: Toast) -> Element {
    let id = toast.id;

    {
        let id = toast.id;
        use_effect(move || {
            spawn(async move {
                gloo_timers::future::TimeoutFuture::new(5000).await;
                remove_toast(id);
            });
        });
    }

    let bg = match toast.level {
        ToastLevel::Error => "#fee2e2",
        ToastLevel::Success => "#dcfce7",
        ToastLevel::Info => "#dbeafe",
    };
    let border = match toast.level {
        ToastLevel::Error => "#ef4444",
        ToastLevel::Success => "#22c55e",
        ToastLevel::Info => "#3b82f6",
    };
    let text = match toast.level {
        ToastLevel::Error => "#991b1b",
        ToastLevel::Success => "#166534",
        ToastLevel::Info => "#1e40af",
    };

    rsx! {
        div {
            style: "background: {bg}; border-left: 4px solid {border}; color: {text}; padding: 12px 16px; border-radius: 6px; min-width: 280px; max-width: 400px; font-size: 0.9rem; box-shadow: 0 2px 8px rgba(0,0,0,0.15); display: flex; justify-content: space-between; align-items: center;",
            span { "{toast.message}" }
            button {
                onclick: move |_| remove_toast(id),
                style: "background: none; border: none; cursor: pointer; font-size: 1.1rem; color: {text}; margin-left: 8px; padding: 0 4px;",
                "×"
            }
        }
    }
}
