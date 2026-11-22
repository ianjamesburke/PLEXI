// Pipe Peer — directed pipe POC for the Plexi WASM runtime (stint 1681).
//
// A controller opens a directed pipe named "ctl" to this plugin and drives it
// with JSON commands on that pipe:
//
//   {"open": NAME, "target": PANE}   open-directed NAME to PANE
//   {"send": NAME, "msg": VALUE}     send VALUE on NAME's current handle
//   {"send_old": NAME}               send on the handle NAME had before its
//                                    latest open (it must be rejected)
//   {"close": NAME}                  close NAME's current handle
//
// Every pipe that is not "ctl" gets its messages echoed back as
// {"echo": PAYLOAD} unless the message is itself an echo. Everything the
// plugin observes is reported to the controller as {"log": LINE} and shown
// in its view.

wit_bindgen::generate!({
    world: "plexi-plugin",
    path: "wit/world.wit",
});

use std::collections::HashMap;

use serde_json::Value;

use exports::plexi::platform::lifecycle::Guest;
use plexi::platform::pipes;
use plexi::platform::types::{
    Alignment, ColumnNode, Effect, IndexedNode, InputEvent, Justify, PipePayload, StateSnapshot,
    TextNode, UiNodeData, UiTree,
};

const CONTROL: &str = "ctl";

#[derive(Default)]
struct PipePeer {
    /// Each pipe name's current handle.
    handles: HashMap<String, u32>,
    /// The handle a name had before its latest open.
    old_handles: HashMap<String, u32>,
    /// Each open request's pipe name.
    requests: HashMap<u32, String>,
    log: Vec<String>,
}

impl PipePeer {
    fn name_of(&self, handle: u32) -> String {
        self.handles
            .iter()
            .chain(self.old_handles.iter())
            .find(|(_, &h)| h == handle)
            .map(|(name, _)| name.clone())
            .unwrap_or_else(|| format!("#{handle}"))
    }

    fn record(&mut self, line: String) {
        if let Some(&ctl) = self.handles.get(CONTROL) {
            let report = serde_json::json!({ "log": line }).to_string();
            if let Err(error) = pipes::send_json(ctl, &report) {
                self.log.push(format!("report-failed:{error}"));
            }
        }
        self.log.push(line);
    }

    fn bind(&mut self, name: String, handle: u32) {
        if let Some(previous) = self.handles.insert(name.clone(), handle) {
            self.old_handles.insert(name, previous);
        }
    }

    fn send(&mut self, label: &str, name: &str, handle: Option<u32>, json: &str) {
        let outcome = match handle {
            Some(handle) => match pipes::send_json(handle, json) {
                Ok(()) => "ok".to_string(),
                Err(error) => format!("err:{error}"),
            },
            None => "err:no handle".to_string(),
        };
        self.record(format!("{label}:{name}:{outcome}"));
    }

    fn command(&mut self, json: &str) {
        let Ok(command) = serde_json::from_str::<Value>(json) else {
            self.record(format!("ctl:invalid:{json}"));
            return;
        };
        let field = |key: &str| command.get(key).and_then(Value::as_str).map(str::to_string);
        if let Some(name) = field("open") {
            let target = command.get("target").and_then(Value::as_u64);
            match target.map(|target| pipes::open_directed(&name, target)) {
                Some(Ok(request)) => {
                    self.requests.insert(request, name);
                }
                Some(Err(error)) => self.record(format!("open:{name}:err:{error}")),
                None => self.record(format!("open:{name}:err:no target")),
            }
        } else if let Some(name) = field("send") {
            let msg = command.get("msg").cloned().unwrap_or(Value::Null);
            let handle = self.handles.get(&name).copied();
            self.send(
                "send",
                &name,
                handle,
                &serde_json::json!({ "msg": msg }).to_string(),
            );
        } else if let Some(name) = field("send_old") {
            let handle = self.old_handles.get(&name).copied();
            self.send("send_old", &name, handle, "{\"msg\":\"stale\"}");
        } else if let Some(name) = field("close") {
            let outcome = match self.handles.remove(&name) {
                Some(handle) => match pipes::close(handle) {
                    Ok(()) => "ok".to_string(),
                    Err(error) => format!("err:{error}"),
                },
                None => "err:no handle".to_string(),
            };
            self.record(format!("close:{name}:{outcome}"));
        } else {
            self.record(format!("ctl:unknown:{json}"));
        }
    }

    fn handle(&mut self, event: InputEvent) -> Vec<Effect> {
        match event {
            InputEvent::PipeIncoming(incoming) => {
                self.bind(incoming.id.clone(), incoming.handle);
                self.record(format!("incoming:{}:{}", incoming.id, incoming.peer_pane));
            }
            InputEvent::PipeOpened(opened) => {
                let name = self
                    .requests
                    .remove(&opened.request)
                    .unwrap_or_else(|| format!("request#{}", opened.request));
                self.bind(name.clone(), opened.handle);
                self.record(format!("opened:{name}"));
                self.send(
                    "hello",
                    &name,
                    Some(opened.handle),
                    "{\"msg\":\"from-wasm\"}",
                );
            }
            InputEvent::PipeOpenRefused(refused) => {
                let name = self
                    .requests
                    .remove(&refused.request)
                    .unwrap_or_else(|| format!("request#{}", refused.request));
                self.record(format!("refused:{name}:{}", refused.reason));
            }
            InputEvent::PipeMessage(message) => {
                let PipePayload::Json(json) = message.payload else {
                    return vec![];
                };
                let name = self.name_of(message.handle);
                if name == CONTROL {
                    self.command(&json);
                } else {
                    let Ok(payload) = serde_json::from_str::<Value>(&json) else {
                        self.record(format!("rx:{name}:invalid:{json}"));
                        return vec![];
                    };
                    self.record(format!("rx:{name}:{payload}"));
                    if payload.get("echo").is_none() {
                        let reply = serde_json::json!({ "echo": payload }).to_string();
                        self.send("echo", &name, Some(message.handle), &reply);
                    }
                }
            }
            InputEvent::PipeClosed(handle) => {
                let name = self.name_of(handle);
                if self.handles.get(&name) == Some(&handle) {
                    self.handles.remove(&name);
                    self.old_handles.insert(name.clone(), handle);
                }
                self.record(format!("closed:{name}"));
            }
            _ => {}
        }
        vec![]
    }

    fn build_tree(&self) -> UiTree {
        let mut nodes: Vec<IndexedNode> = self
            .log
            .iter()
            .enumerate()
            .map(|(i, line)| IndexedNode {
                id: i as u32,
                key: format!("log-{i}"),
                data: UiNodeData::Text(TextNode {
                    text: line.clone(),
                    size: None,
                    bold: false,
                    color: None,
                    truncate: false,
                    align: Alignment::Start,
                }),
            })
            .collect();
        let root = nodes.len() as u32;
        nodes.push(IndexedNode {
            id: root,
            key: "root".to_string(),
            data: UiNodeData::Column(ColumnNode {
                children: (0..root).collect(),
                gap: 4.0,
                align: Alignment::Start,
                justify: Justify::Start,
                grow: true,
            }),
        });
        UiTree { root, nodes }
    }
}

struct Component;

static mut APP: Option<PipePeer> = None;

fn app() -> &'static mut PipePeer {
    unsafe { (*core::ptr::addr_of_mut!(APP)).get_or_insert_with(PipePeer::default) }
}

impl Guest for Component {
    fn init(_state: StateSnapshot, _size: (f32, f32), _args: Vec<String>) -> Vec<Effect> {
        app();
        vec![]
    }

    fn update(event: InputEvent) -> Vec<Effect> {
        app().handle(event)
    }

    fn view() -> UiTree {
        app().build_tree()
    }
}

export!(Component);
