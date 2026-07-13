//! Rust authoring API for Plexi WASM component apps.
//!
//! Implement [`App`], then call [`export_app!`] once with an app constructor.

wit_bindgen::generate!({
    world: "plexi-app",
    path: "wit",
});

/// Generated WIT bindings for advanced host-interface use.
pub mod bindings {
    pub use crate::{exports, plexi};
}

pub use bindings::plexi::platform::types::*;

/// Values passed to [`App::init`].
pub struct InitContext {
    /// State persisted by an earlier app instance.
    pub state: StateSnapshot,
    /// Initial content size in logical pixels.
    pub size: (f32, f32),
    /// Arguments from the app-open command.
    pub args: Vec<String>,
}

impl InitContext {
    /// Creates an initialization context.
    #[must_use]
    pub fn new(state: StateSnapshot, size: (f32, f32), args: Vec<String>) -> Self {
        Self { state, size, args }
    }

    /// Returns the initial persisted value for `key`.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&[u8]> {
        self.state
            .entries
            .iter()
            .find_map(|(entry_key, value)| (entry_key == key).then_some(value.as_slice()))
    }
}

/// Lifecycle implemented by a standard Plexi WASM app.
pub trait App {
    /// Initializes app state and returns startup effects.
    fn init(&mut self, _context: InitContext) -> Vec<Effect> {
        Vec::new()
    }

    /// Applies one host event and returns effects.
    fn update(&mut self, event: InputEvent) -> Vec<Effect>;

    /// Builds the current UI tree. This method must not call host imports.
    fn view(&self) -> UiTree;
}

struct PlexiComponent;
static mut PLEXI_APP: Option<Box<dyn App>> = None;

fn app() -> &'static mut dyn App {
    unsafe {
        (*core::ptr::addr_of_mut!(PLEXI_APP))
            .as_deref_mut()
            .expect("Plexi calls init before update or view")
    }
}

unsafe extern "Rust" {
    fn __plexi_create_app() -> Box<dyn App>;
}

impl bindings::exports::plexi::platform::lifecycle::Guest for PlexiComponent {
    fn init(state: StateSnapshot, size: (f32, f32), args: Vec<String>) -> Vec<Effect> {
        unsafe {
            PLEXI_APP = Some(__plexi_create_app());
        }
        app().init(InitContext::new(state, size, args))
    }

    fn update(event: InputEvent) -> Vec<Effect> {
        app().update(event)
    }

    fn view() -> UiTree {
        app().view()
    }
}

export!(PlexiComponent);

/// Exports an [`App`] as the `plexi-app` WIT world.
#[macro_export]
macro_rules! export_app {
    ($constructor:expr) => {
        /// Creates the app instance used by the generated WIT lifecycle export.
        #[no_mangle]
        pub fn __plexi_create_app() -> Box<dyn $crate::App> {
            Box::new($constructor)
        }
    };
}

/// Persistent state imports.
pub mod state {
    pub use crate::bindings::plexi::platform::host_state::{
        delete, get, list_prefix, set, snapshot,
    };
}

/// Host logging imports.
pub mod log {
    pub use crate::bindings::plexi::platform::host_log::{debug, error, info, warn};
}

/// Constructors for host effects.
pub mod effects {
    use super::*;

    #[must_use]
    pub fn file_read(path: impl Into<String>) -> Effect {
        Effect::FileRead(FileReadEffect { path: path.into() })
    }
    #[must_use]
    pub fn file_write(path: impl Into<String>, content: impl AsRef<[u8]>) -> Effect {
        Effect::FileWrite(FileWriteEffect {
            path: path.into(),
            content: content.as_ref().to_vec(),
        })
    }
    #[must_use]
    pub fn http_fetch(url: impl Into<String>) -> Effect {
        Effect::HttpFetch(HttpFetchEffect {
            url: url.into(),
            method: "GET".into(),
            headers: vec![],
            body: None,
        })
    }
    #[must_use]
    pub fn ai_query(effect: AiQueryEffect) -> Effect {
        Effect::AiQuery(effect)
    }
    #[must_use]
    pub fn declare_event_streams(streams: Vec<EventStreamDecl>) -> Effect {
        Effect::DeclareEventStreams(DeclareEventStreamsEffect { streams })
    }
    #[must_use]
    pub fn emit_event(effect: EmitEventEffect) -> Effect {
        Effect::EmitEvent(effect)
    }
    #[must_use]
    pub fn set_timer(id: u32, delay_ms: u32, repeat: bool) -> Effect {
        Effect::SetTimer(TimerEffect {
            id,
            delay_ms,
            repeat,
        })
    }
    #[must_use]
    pub fn cancel_timer(id: u32) -> Effect {
        Effect::CancelTimer(id)
    }
    #[must_use]
    pub fn get_system_stats() -> Effect {
        Effect::GetSystemStats
    }
    #[must_use]
    pub fn set_title(title: impl Into<String>) -> Effect {
        Effect::SetTitle(title.into())
    }
    #[must_use]
    pub fn set_status(status: impl Into<String>) -> Effect {
        Effect::SetStatus(status.into())
    }
    #[must_use]
    pub fn close_self() -> Effect {
        Effect::CloseSelf
    }
    #[must_use]
    pub fn request_capability(capability: impl Into<String>) -> Effect {
        Effect::RequestCapability(capability.into())
    }
    #[must_use]
    pub fn clipboard_read() -> Effect {
        Effect::ClipboardRead
    }
    #[must_use]
    pub fn clipboard_write(text: impl Into<String>) -> Effect {
        Effect::ClipboardWrite(text.into())
    }
    #[must_use]
    pub fn notify(title: impl Into<String>, body: impl Into<String>) -> Effect {
        Effect::Notify(NotificationEffect {
            title: title.into(),
            body: body.into(),
            icon: None,
        })
    }
    #[must_use]
    pub fn spawn(app_id: impl Into<String>) -> Effect {
        Effect::Spawn(SpawnEffect {
            app_id: app_id.into(),
            layout: None,
            args: vec![],
        })
    }
}

/// Arena UI tree builder. Each method maps one-to-one to a WIT `ui-node-data` variant.
pub mod ui {
    use super::*;

    /// Builds indexed nodes and assigns stable numeric IDs in insertion order.
    #[derive(Default)]
    pub struct Tree {
        nodes: Vec<IndexedNode>,
    }

    impl Tree {
        #[must_use]
        pub fn new() -> Self {
            Self::default()
        }
        fn push(&mut self, key: impl Into<String>, data: UiNodeData) -> u32 {
            let id = self.nodes.len() as u32;
            self.nodes.push(IndexedNode {
                id,
                key: key.into(),
                data,
            });
            id
        }
        pub fn empty(&mut self, key: impl Into<String>) -> u32 {
            self.push(key, UiNodeData::Empty)
        }
        pub fn text(&mut self, key: impl Into<String>, text: impl Into<String>) -> u32 {
            self.push(
                key,
                UiNodeData::Text(TextNode {
                    text: text.into(),
                    size: None,
                    bold: false,
                    color: None,
                    truncate: false,
                    align: Alignment::Start,
                }),
            )
        }
        pub fn button(
            &mut self,
            key: impl Into<String>,
            label: impl Into<String>,
            on_click: impl Into<String>,
        ) -> u32 {
            self.push(
                key,
                UiNodeData::Button(ButtonNode {
                    label: label.into(),
                    on_click: on_click.into(),
                    style: ButtonStyle::Primary,
                    disabled: false,
                }),
            )
        }
        pub fn text_input(
            &mut self,
            key: impl Into<String>,
            value: impl Into<String>,
            placeholder: impl Into<String>,
            on_change: impl Into<String>,
            on_submit: impl Into<String>,
        ) -> u32 {
            self.push(
                key,
                UiNodeData::TextInput(TextInputNode {
                    value: value.into(),
                    placeholder: placeholder.into(),
                    on_change: on_change.into(),
                    on_submit: on_submit.into(),
                    password: false,
                }),
            )
        }
        pub fn row(
            &mut self,
            key: impl Into<String>,
            children: impl IntoIterator<Item = u32>,
        ) -> u32 {
            self.push(
                key,
                UiNodeData::Row(RowNode {
                    children: children.into_iter().collect(),
                    gap: 8.0,
                    align: Alignment::Start,
                    grow: false,
                }),
            )
        }
        pub fn column(
            &mut self,
            key: impl Into<String>,
            children: impl IntoIterator<Item = u32>,
        ) -> u32 {
            self.push(
                key,
                UiNodeData::Column(ColumnNode {
                    children: children.into_iter().collect(),
                    gap: 8.0,
                    align: Alignment::Start,
                    grow: false,
                }),
            )
        }
        pub fn progress_bar(&mut self, key: impl Into<String>, value: f32, max: f32) -> u32 {
            self.push(
                key,
                UiNodeData::ProgressBar(ProgressBarNode {
                    value,
                    max,
                    color: None,
                    label: None,
                }),
            )
        }
        pub fn badge(&mut self, key: impl Into<String>, text: impl Into<String>) -> u32 {
            self.push(
                key,
                UiNodeData::Badge(BadgeNode {
                    text: text.into(),
                    color: BadgeColor::Neutral,
                }),
            )
        }
        pub fn list_view(
            &mut self,
            key: impl Into<String>,
            items: impl IntoIterator<Item = u32>,
            selected: Option<u32>,
            on_select: Option<&str>,
        ) -> u32 {
            self.push(
                key,
                UiNodeData::ListView(ListNode {
                    items: items.into_iter().collect(),
                    selected,
                    on_select: on_select.map(str::to_owned),
                }),
            )
        }
        pub fn scroll(&mut self, key: impl Into<String>, child: u32, horizontal: bool) -> u32 {
            self.push(key, UiNodeData::Scroll(ScrollNode { child, horizontal }))
        }
        pub fn padding(&mut self, key: impl Into<String>, child: u32, amount: f32) -> u32 {
            self.push(
                key,
                UiNodeData::Padding(PaddingNode {
                    child,
                    top: amount,
                    right: amount,
                    bottom: amount,
                    left: amount,
                }),
            )
        }
        pub fn canvas(
            &mut self,
            key: impl Into<String>,
            width: f32,
            height: f32,
            commands: impl IntoIterator<Item = CanvasCommand>,
        ) -> u32 {
            self.push(
                key,
                UiNodeData::Canvas(CanvasNode {
                    width,
                    height,
                    grow: false,
                    commands: commands.into_iter().collect(),
                }),
            )
        }
        pub fn divider(&mut self, key: impl Into<String>) -> u32 {
            self.push(key, UiNodeData::Divider)
        }
        pub fn space(&mut self, key: impl Into<String>, amount: f32) -> u32 {
            self.push(key, UiNodeData::Space(amount))
        }
        pub fn surface(
            &mut self,
            key: impl Into<String>,
            width: u32,
            height: u32,
            texture_handle: Option<u64>,
        ) -> u32 {
            self.push(
                key,
                UiNodeData::Surface(SurfaceNode {
                    width,
                    height,
                    texture_handle,
                }),
            )
        }
        #[must_use]
        pub fn finish(self, root: u32) -> UiTree {
            UiTree {
                root,
                nodes: self.nodes,
            }
        }
    }
}
