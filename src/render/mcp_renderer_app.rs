//! Native, broker-admitted MCP explorer. Transport is host-owned and serviced off paint.
use super::{descriptor_widgets, mcp_descriptor};
use crate::app::app_trait::{App, AppRenderContext, KeyDisposition};
use crate::app::plexi_descriptor::Command;
use crate::broker::gate::{Admission, AdmitRequest, PermissionMonitor};
use crate::broker::{ActorScope, ActorType, TargetType};
use crate::host::mcp_client::{self, McpConnection, McpLine, McpServerConfig, McpServerRegistry};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone)]
struct Entry {
    command: Command,
    schema: Value,
    target: String,
}
struct Pending {
    method: String,
    started: Instant,
    pages: usize,
}

pub struct McpRendererApp {
    server: String,
    workspace: PathBuf,
    identity: Option<(u64, u64)>,
    monitor: Arc<PermissionMonitor>,
    connection: Option<McpConnection>,
    receiver: mcp_client::InboundReceiver,
    sender: mcp_client::InboundSender,
    wake: Option<Arc<dyn crate::app::ui_mailbox::UiWake>>,
    pending: BTreeMap<u64, Pending>,
    next_id: u64,
    entries: [Vec<Entry>; 3],
    tab: usize,
    selected: usize,
    last_selection: Option<(usize, usize)>,
    form: bool,
    focus: bool,
    values: HashMap<String, String>,
    result: String,
    connect_requested: bool,
    config: Option<McpServerConfig>,
    valid_server_id: bool,
}
impl McpRendererApp {
    pub fn new(args: &[String], workspace: PathBuf) -> Self {
        let (sender, receiver) = mcp_client::inbound_channel();
        let valid = args.len() == 1 && !args[0].is_empty();
        Self {
            server: args.first().cloned().unwrap_or_default(),
            workspace,
            identity: None,
            monitor: PermissionMonitor::for_profile(&crate::config::config_dir()),
            connection: None,
            receiver,
            sender,
            wake: None,
            pending: BTreeMap::new(),
            next_id: 1,
            entries: Default::default(),
            tab: 0,
            selected: 0,
            last_selection: None,
            form: false,
            focus: false,
            values: HashMap::new(),
            result: if valid {
                "Connect to discover tools, resources and prompts.".into()
            } else {
                "Expected one configured MCP server id, not a command.".into()
            },
            connect_requested: valid,
            config: None,
            valid_server_id: valid,
        }
    }
    fn fail(&mut self, error: impl ToString) {
        self.result = error.to_string();
        log::warn!("mcp_renderer: server={} error={}", self.server, self.result);
    }
    fn authorize(&self, action: &str, params: &Value) -> Result<(), String> {
        let (pane, context) = self
            .identity
            .ok_or("MCP pane is not bound to a host context")?;
        let config = self.config.as_ref().ok_or("MCP server is not resolved")?;
        // Include the configured executable identity in every exact grant binding.
        use sha2::{Digest, Sha256};
        let identity =
            json!({"command":config.command,"args":config.args,"env":config.env,"cwd":config.cwd})
                .to_string();
        let config_hash = format!("{:x}", Sha256::digest(identity.as_bytes()));
        let input =
            json!({"server":self.server,"config_hash":config_hash,"params":params}).to_string();
        let actor = format!("app:mcp-renderer:{}", self.server);
        let tool = format!("mcp.{}.{}", self.server, action);
        let call = format!("mcp_{}", uuid::Uuid::new_v4());
        match self.monitor.admit(AdmitRequest {
            call_id: &call,
            tool: &tool,
            input_json: &input,
            actor_type: ActorType::App,
            actor_id: &actor,
            actor_scope: ActorScope::BuiltIn,
            trust_origin: "host",
            workspace_root: &self.workspace,
            context_id: context,
            package_id: "mcp-renderer",
            instance_id: pane,
            target_type: TargetType::McpTool,
        }) {
            Admission::Required { pending_request_id } => Err(format!(
                "Permission required: {pending_request_id}. Approve in Needs You, then retry."
            )),
            Admission::Denied { code } => Err(code.into()),
            Admission::Proceed {
                grant_id,
                fingerprint,
                resource_id,
                ..
            } => {
                self.monitor.note_use(
                    &actor,
                    &call,
                    &grant_id,
                    &fingerprint,
                    resource_id.as_deref().unwrap_or(""),
                    "",
                )?;
                self.monitor.consume_once(&grant_id, None);
                Ok(())
            }
        }
    }
    fn connect(&mut self) -> Result<(), String> {
        if !self.valid_server_id {
            return Err("Expected one configured MCP server id, not a command.".into());
        }
        if !crate::release::feature_enabled(crate::release::ReleaseFeature::McpClient) {
            return Err(crate::release::feature_unavailable_message(
                crate::release::ReleaseFeature::McpClient,
            ));
        }
        let registry =
            McpServerRegistry::load(&crate::config::config_dir()).map_err(|e| e.to_string())?;
        self.config = Some(
            registry
                .resolve(&self.server)
                .map_err(|e| e.to_string())?
                .clone(),
        );
        self.authorize("connect", &json!({}))?;
        let wake = self.wake.clone().ok_or("Host wake is not ready")?;
        // A reconnect gets a fresh feed: a late close from the old child must
        // never tear down its replacement.
        self.connection = None;
        let (sender, receiver) = mcp_client::inbound_channel();
        self.sender = sender;
        self.receiver = receiver;
        self.connection = Some(
            McpConnection::spawn(
                &self.server,
                self.config.as_ref().ok_or("Missing config")?,
                &self.workspace,
                self.sender.clone(),
                Arc::new(move || wake.wake("mcp_renderer")),
            )
            .map_err(|e| format!("Connect: {e}"))?,
        );
        self.entries = Default::default();
        self.pending.clear();
        self.send("initialize",json!({"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"plexi","version":env!("CARGO_PKG_VERSION")}}),0)?;
        self.result = "Connecting…".into();
        log::info!("mcp_renderer: connecting server={}", self.server);
        Ok(())
    }
    fn send(&mut self, method: &str, params: Value, pages: usize) -> Result<(), String> {
        let id = self.next_id;
        self.next_id += 1;
        self.connection
            .as_ref()
            .ok_or("Not connected")?
            .send(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}).to_string())?;
        self.pending.insert(
            id,
            Pending {
                method: method.into(),
                started: Instant::now(),
                pages,
            },
        );
        Ok(())
    }
    fn receive(&mut self, body: &str) -> Result<(), String> {
        let message: Value =
            serde_json::from_str(body).map_err(|e| format!("Invalid MCP JSON: {e}"))?;
        let Some(id) = message["id"].as_u64() else {
            return Ok(());
        };
        let Some(pending) = self.pending.remove(&id) else {
            return Ok(());
        };
        if let Some(error) = message.get("error") {
            return Err(format!("{}: {}", pending.method, error));
        }
        let result = message
            .get("result")
            .ok_or("MCP response has neither result nor error")?;
        match pending.method.as_str() {
            "initialize" => {
                self.connection.as_ref().ok_or("Disconnected")?.send(
                    &json!({"jsonrpc":"2.0","method":"notifications/initialized"}).to_string(),
                )?;
                for (cap, method) in [
                    ("tools", "tools/list"),
                    ("resources", "resources/list"),
                    ("prompts", "prompts/list"),
                ] {
                    if result["capabilities"].get(cap).is_some() {
                        self.send(method, json!({}), 0)?;
                    }
                }
                self.result = "Connected. Select an endpoint.".into();
            }
            "tools/list" | "resources/list" | "prompts/list" => {
                let (tab, key) = match pending.method.as_str() {
                    "tools/list" => (0, "tools"),
                    "resources/list" => (1, "resources"),
                    _ => (2, "prompts"),
                };
                let items = result[key]
                    .as_array()
                    .ok_or_else(|| format!("{key} must be an array"))?;
                if self.entries[tab].len() + items.len() > 4096 {
                    return Err("MCP discovery exceeds 4096 entries".into());
                }
                for item in items {
                    let (tool, target) = if tab == 0 {
                        (item.clone(), item["name"].as_str().unwrap_or("").to_owned())
                    } else if tab == 1 {
                        (
                            json!({"name":item["name"],"description":item.get("description"),"inputSchema":{"type":"object","properties":{}}}),
                            item["uri"].as_str().ok_or("Resource has no URI")?.into(),
                        )
                    } else {
                        let mut props = serde_json::Map::new();
                        let mut required = Vec::new();
                        if let Some(args) = item["arguments"].as_array() {
                            for arg in args {
                                let name =
                                    arg["name"].as_str().ok_or("Prompt argument has no name")?;
                                props.insert(
                                    name.into(),
                                    json!({"type":"string","description":arg.get("description")}),
                                );
                                if arg["required"] == true {
                                    required.push(name);
                                }
                            }
                        }
                        (
                            json!({"name":item["name"],"description":item.get("description"),"inputSchema":{"type":"object","properties":props,"required":required}}),
                            item["name"].as_str().ok_or("Prompt has no name")?.into(),
                        )
                    };
                    self.entries[tab].push(Entry {
                        command: mcp_descriptor::tool_command(&tool)?,
                        schema: tool["inputSchema"].clone(),
                        target,
                    });
                }
                if let Some(cursor) = result["nextCursor"].as_str() {
                    if pending.pages >= 31 {
                        return Err("MCP discovery pagination limit reached".into());
                    }
                    self.send(&pending.method, json!({"cursor":cursor}), pending.pages + 1)?;
                }
            }
            _ => self.result = serde_json::to_string_pretty(result).map_err(|e| e.to_string())?,
        }
        Ok(())
    }
    fn open_selected(&mut self) {
        if let Some(entry) = self.entries[self.tab].get(self.selected) {
            self.values = entry
                .command
                .args
                .iter()
                .filter_map(|f| {
                    f.default
                        .as_ref()
                        .map(|v| (f.name.clone(), mcp_descriptor::display_value(v)))
                })
                .collect();
            self.form = true;
            self.focus = true;
        }
    }
    fn submit(&mut self) -> Result<(), String> {
        if !self.pending.is_empty() {
            return Err("Wait for the pending MCP request before retrying.".into());
        }
        let entry = self.entries[self.tab]
            .get(self.selected)
            .ok_or("Select an endpoint")?
            .clone();
        let args = mcp_descriptor::arguments(&entry.command, &entry.schema, &self.values)?;
        let (method, params, action) = match self.tab {
            0 => (
                "tools/call",
                json!({"name":entry.target,"arguments":args}),
                entry.target,
            ),
            1 => (
                "resources/read",
                json!({"uri":entry.target}),
                "resources.read".into(),
            ),
            _ => (
                "prompts/get",
                json!({"name":entry.target,"arguments":args}),
                "prompts.get".into(),
            ),
        };
        self.authorize(&action, &params)?;
        self.send(method, params, 0)?;
        self.result = "Waiting for response…".into();
        log::info!(
            "mcp_renderer: dispatched server={} method={method}",
            self.server
        );
        Ok(())
    }
    fn pump(&mut self) -> bool {
        let mut changed = false;
        for _ in 0..32 {
            match self.receiver.try_recv() {
                Ok((_, McpLine::Message(body))) => {
                    changed = true;
                    if let Err(e) = self.receive(&body) {
                        self.fail(e);
                    }
                }
                Ok((_, McpLine::Closed(reason))) => {
                    changed = true;
                    self.connection = None;
                    self.pending.clear();
                    self.fail(format!("Connection closed: {reason}"));
                }
                Err(_) => break,
            }
        }
        if self
            .pending
            .values()
            .any(|p| p.started.elapsed() > Duration::from_secs(60))
        {
            self.pending.clear();
            self.connection = None;
            self.fail("MCP request timed out; reconnect to retry.");
            changed = true;
        }
        changed
    }
}
impl App for McpRendererApp {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn type_id(&self) -> &'static str {
        "mcp-renderer"
    }
    fn display_name(&self) -> String {
        format!("MCP · {}", self.server)
    }
    fn bind_host_identity(&mut self, pane: u64, context: u64) {
        self.identity = Some((pane, context));
    }
    fn service_external(&mut self, wake: &Arc<dyn crate::app::ui_mailbox::UiWake>) -> bool {
        self.wake = Some(wake.clone());
        let mut changed = false;
        if self.connect_requested {
            self.connect_requested = false;
            changed = true;
            if let Err(e) = self.connect() {
                self.fail(e);
            }
        }
        changed | self.pump()
    }
    fn background_tick(&mut self) {
        self.pump();
    }
    fn needs_background_tick(&self) -> bool {
        !self.pending.is_empty()
    }
    fn submit_on_focused_text_input_enter(&self) -> bool {
        self.form
    }
    fn handle_key(&mut self, input: &crate::app::input_router::PlexiInput) -> KeyDisposition {
        use egui::Key;
        let modifiers = input.modifiers();
        if modifiers.command || modifiers.ctrl || modifiers.alt {
            return KeyDisposition::Passthrough;
        }
        if input.key_pressed(Key::Escape) {
            self.form = false;
            return KeyDisposition::Consumed;
        }
        if input.key_pressed(Key::Enter) {
            if self.form {
                if let Err(e) = self.submit() {
                    self.fail(e);
                }
            } else if self.connection.is_none() && self.entries[self.tab].is_empty() {
                self.connect_requested = true;
            } else {
                self.open_selected();
            }
            return KeyDisposition::Consumed;
        }
        if !self.form {
            if input.key_pressed(Key::Tab) {
                self.tab = (self.tab + 1) % 3;
                self.selected = 0;
                return KeyDisposition::Consumed;
            }
            let count = self.entries[self.tab].len();
            if count > 0 && (input.key_pressed(Key::J) || input.key_pressed(Key::ArrowDown)) {
                self.selected = (self.selected + 1) % count;
                return KeyDisposition::Consumed;
            }
            if count > 0 && (input.key_pressed(Key::K) || input.key_pressed(Key::ArrowUp)) {
                self.selected = (self.selected + count - 1) % count;
                return KeyDisposition::Consumed;
            }
        }
        KeyDisposition::Passthrough
    }
    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &AppRenderContext<'_>,
        _click: Option<crate::host::pane::PendingPaneClick>,
    ) {
        if !self.pending.is_empty() {
            ui.ctx().request_repaint_after(Duration::from_secs(1));
        }
        crate::ui::app_chrome::AppChrome::new(ctx.colors).paint_app_bar(
            ui,
            &self.display_name(),
            "Explore tools, resources and prompts",
        );
        ui.horizontal(|ui| {
            for (i, label) in ["Tools", "Resources", "Prompts"].into_iter().enumerate() {
                if ui.selectable_label(self.tab == i, label).clicked() {
                    self.tab = i;
                    self.selected = 0;
                    self.form = false;
                }
            }
            if ui
                .button(if self.connection.is_some() {
                    "Reconnect"
                } else {
                    "Connect / retry"
                })
                .clicked()
            {
                self.connect_requested = true;
            }
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt(("mcp", ctx.pane_id))
            .show(ui, |ui| {
                if self.form {
                    if ui.button("← Back").clicked() {
                        self.form = false;
                    }
                    if let Some(entry) = self.entries[self.tab].get(self.selected) {
                        ui.heading(&entry.command.name);
                        descriptor_widgets::form_fields(
                            ui,
                            ctx.colors,
                            &entry.command.args,
                            &mut self.values,
                            self.focus,
                            ctx.pane_id,
                        );
                        self.focus = false;
                        if ui.button("Run / retry").clicked() {
                            if let Err(e) = self.submit() {
                                self.fail(e);
                            }
                        }
                    }
                } else {
                    if let Some(index) = descriptor_widgets::command_list(
                        ui,
                        ctx.colors,
                        self.entries[self.tab].iter().map(|e| &e.command),
                        "",
                        self.selected,
                        self.last_selection != Some((self.tab, self.selected)),
                    ) {
                        self.selected = index;
                        self.open_selected();
                    }
                    self.last_selection = Some((self.tab, self.selected));
                    if self.entries[self.tab].is_empty() {
                        ui.label("No endpoints discovered in this view.");
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.strong("Result");
                    if ui.button("Copy").clicked() {
                        ui.ctx().copy_text(self.result.clone());
                    }
                });
                ui.add(
                    egui::Label::new(egui::RichText::new(&self.result).monospace())
                        .wrap()
                        .selectable(true),
                );
                ui.separator();
                ui.small("j / k Navigate · Enter Open / run · Tab Views / fields · Esc Back");
            });
    }
    fn semantic_state(&self) -> Option<Value> {
        Some(
            json!({"server":self.server,"tools":self.entries[0].iter().map(|e|&e.command.name).collect::<Vec<_>>(),"resources":self.entries[1].len(),"prompts":self.entries[2].len(),"result":self.result,"form":self.form}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::gate::ApprovalChoice;
    struct Wake;
    impl crate::app::ui_mailbox::UiWake for Wake {
        fn wake(&self, _source: &'static str) {}
    }
    fn configured(dir: &std::path::Path) -> McpRendererApp {
        let mut app = McpRendererApp::new(&["fixture".into()], dir.to_path_buf());
        app.bind_host_identity(91, 7);
        app.config = Some(McpServerConfig {
            command: "/bin/sh".into(),
            args: vec![],
            env: Default::default(),
            cwd: None,
        });
        app.connect_requested = false;
        app
    }
    #[test]
    fn ungranted_mcp_call_files_needs_you_and_once_allows_only_retry() {
        let dir = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(dir.path().to_path_buf());
        let _shared = crate::config::set_test_shared_dir(dir.path().join("shared"));
        let app = configured(dir.path());
        let params = json!({"path":"/tmp/example"});
        assert!(app
            .authorize("read_text_file", &params)
            .unwrap_err()
            .contains("Permission required"));
        assert_eq!(app.monitor.all_needs_you().len(), 1);
        let pending = app.monitor.list_pending();
        app.monitor
            .approve_pending(&pending[0].pending_request_id, ApprovalChoice::Once)
            .unwrap();
        app.authorize("read_text_file", &params).unwrap();
        assert!(app.authorize("read_text_file", &params).is_err());
        assert!(app.authorize("write_file", &params).is_err());
    }
    #[test]
    fn mcp_session_and_always_grants_remain_bound_to_server_and_context() {
        for choice in [ApprovalChoice::Session, ApprovalChoice::Always] {
            let dir = tempfile::tempdir().unwrap();
            let _profile = crate::config::set_test_profile_dir(dir.path().to_path_buf());
            let _shared = crate::config::set_test_shared_dir(dir.path().join("shared"));
            let mut app = configured(dir.path());
            let params = json!({"message":"hello"});
            assert!(app.authorize("echo", &params).is_err());
            let pending = app.monitor.list_pending();
            app.monitor
                .approve_pending(&pending[0].pending_request_id, choice)
                .unwrap();
            app.authorize("echo", &params).unwrap();
            app.authorize("echo", &params).unwrap();
            assert!(app
                .authorize("echo", &json!({"message":"changed"}))
                .is_err());
            app.bind_host_identity(91, 8);
            assert!(app.authorize("echo", &params).is_err());
            app.bind_host_identity(91, 7);
            app.config
                .as_mut()
                .unwrap()
                .args
                .push("changed-command".into());
            assert!(app.authorize("echo", &params).is_err());
        }
    }
    #[test]
    fn mcp_builtin_binds_identity_and_requests_consent_without_paint() {
        let mut h = crate::testing::HostHarness::new();
        let _experiments =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::McpClient]);
        std::fs::write(
            crate::config::config_dir().join("mcp_servers.toml"),
            "[servers.fixture]\ncommand = 'not-started-without-consent'\n",
        )
        .unwrap();
        let cwd = h.workspace_root();
        h.app
            .launch_app_by_id_with_layout("mcp-renderer", None, &["fixture".into()], Some(cwd))
            .unwrap();
        h.hidden_frame();
        let monitor = PermissionMonitor::for_profile(&crate::config::config_dir());
        let pending = monitor.list_pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].tool, "mcp.fixture.connect");
        for pane in h.app.windows[h.app.active_window].panes.values_mut() {
            if let Some(pane) = pane.as_app_mut() {
                if let crate::host::pane::AppRuntime::Builtin(app) = &mut pane.runtime {
                    if let Some(app) = app.as_any_mut().downcast_mut::<McpRendererApp>() {
                        assert_eq!(app.identity.unwrap().0, pane.id);
                        assert!(app.connection.is_none());
                        assert!(app.result.contains("Permission required"));
                    }
                }
            }
        }
    }
    #[cfg(unix)]
    #[test]
    fn stdio_mock_discovers_and_calls_all_views_off_paint() {
        let dir = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(dir.path().to_path_buf());
        let _shared = crate::config::set_test_shared_dir(dir.path().join("shared"));
        let _experiments =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::McpClient]);
        let script = dir.path().join("mock.sh");
        std::fs::write(&script,r#"while IFS= read -r line; do
id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
case "$line" in
*'"method":"initialize"'*) result='{"capabilities":{"tools":{},"resources":{},"prompts":{}},"protocolVersion":"2024-11-05","serverInfo":{"name":"fixture","version":"1"}}';;
*'"method":"tools/list"'*) result='{"tools":[{"name":"echo","description":"Echo a message","inputSchema":{"type":"object","properties":{"message":{"type":"string"}},"required":["message"]}}]}';;
*'"method":"resources/list"'*) result='{"resources":[{"name":"Example","uri":"fixture://example"}]}';;
*'"method":"prompts/list"'*) result='{"prompts":[{"name":"greet","arguments":[{"name":"name","required":true}]}]}';;
*'"method":"notifications/initialized"'*) continue;;
*) result='{"content":[{"type":"text","text":"fixture success"}]}';;
esac
printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$id" "$result"
done
"#).unwrap();
        std::fs::write(
            dir.path().join("mcp_servers.toml"),
            format!(
                "[servers.fixture]\ncommand = '/bin/sh'\nargs = ['{}']\n",
                script.display()
            ),
        )
        .unwrap();
        let mut app = configured(dir.path());
        let wake: Arc<dyn crate::app::ui_mailbox::UiWake> = Arc::new(Wake);
        app.wake = Some(wake.clone());
        assert!(app.connect().is_err());
        let pending = app.monitor.list_pending();
        app.monitor
            .approve_pending(&pending[0].pending_request_id, ApprovalChoice::Session)
            .unwrap();
        app.connect().unwrap();
        let timeout = crate::testing::load_aware_timeout(Duration::from_secs(10));
        let deadline = Instant::now() + timeout;
        while app.entries.iter().any(Vec::is_empty) {
            app.service_external(&wake);
            assert!(Instant::now() < deadline, "{}", app.result);
            std::thread::sleep(Duration::from_millis(10));
        }
        for tab in 0..3 {
            app.tab = tab;
            app.selected = 0;
            app.open_selected();
            app.values.insert("message".into(), "hello".into());
            app.values.insert("name".into(), "Ian".into());
            assert!(app.submit().is_err());
            let pending = app.monitor.list_pending();
            let last = pending
                .iter()
                .find(|p| p.tool != "mcp.fixture.connect")
                .unwrap();
            app.monitor
                .approve_pending(&last.pending_request_id, ApprovalChoice::Once)
                .unwrap();
            app.submit().unwrap();
            while !app.pending.is_empty() {
                app.service_external(&wake);
                assert!(Instant::now() < deadline, "{}", app.result);
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(app.result.contains("fixture success"), "{}", app.result);
        }
    }
    #[test]
    fn screenshot_mcp_visualizer_forms() {
        let mut h = crate::ui_tests::PlexiUiHarness::new_sized_ppp(900.0, 760.0, 1.0);
        let workspace = tempfile::tempdir().unwrap();
        let cwd = workspace.path().to_path_buf();
        h.with_app_mut(|host| {
            host.sidebar_visible = false;
            let mut app = configured(&cwd);
            let value: Value = serde_json::from_str(include_str!(
                "../host/testdata/mcp/filesystem_tools_list.json"
            ))
            .unwrap();
            for tool in value["result"]["tools"].as_array().unwrap() {
                app.entries[0].push(Entry {
                    command: mcp_descriptor::tool_command(tool).unwrap(),
                    schema: tool["inputSchema"].clone(),
                    target: tool["name"].as_str().unwrap().into(),
                });
            }
            app.result = "Connected. Select an endpoint.".into();
            host.open_builtin_app_pane(
                Box::new(app),
                crate::app::permissions::AppPermissions::builtin(),
                cwd,
                None,
                None,
                None,
            );
        });
        h.run_steps(3);
        h.save_screenshot(&crate::ui_tests::evidence_png!("_list"))
            .unwrap();
        h.harness().key_press(egui::Key::J);
        h.run_steps(2);
        h.harness().key_press(egui::Key::K);
        h.run_steps(2);
        h.harness().key_press(egui::Key::J);
        h.run_steps(2);
        h.harness().key_press(egui::Key::Enter);
        h.run_steps(3);
        h.with_app(|host| {
            let state = host.windows[host.active_window]
                .panes
                .values()
                .filter_map(|pane| pane.as_app())
                .find(|pane| pane.manifest_id == "mcp-renderer")
                .unwrap()
                .runtime
                .semantic_details()
                .unwrap();
            assert_eq!(state["form"], true, "Enter must open the selected form");
        });
        h.save_screenshot(&crate::ui_tests::evidence_png!("_form"))
            .unwrap();
    }
}
