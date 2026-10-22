# MCP visualizer

`plexi app open --mcp <server-id>` and the `mcp:<server-id>` prefix open the
native `mcp-renderer` builtin. The host resolves the id through
`McpServerRegistry` in `src/host/mcp_client.rs`. That module owns the configuration
format and process transport. Launch arguments cannot supply an executable.
`ReleaseFeature::McpClient` gates both opening the app and connecting.

The host binds the pane and context identity before servicing external work.
Connecting requests `mcp.<server>.connect`; tool calls request
`mcp.<server>.<tool>`. Resource reads and prompt calls use
`mcp.<server>.resources.read` and `mcp.<server>.prompts.get`.
`PermissionMonitor` owns admission, Needs You cards, and once/session/always
decisions. A refused operation stays unsent. After desktop approval, the user
retries it. Grants retain the monitor's exact argument, pane, context and
workspace binding, including a hash of the server configuration. A changed
configuration needs a new decision. The hash keeps configured environment
values out of the approval summary.

The app initializes MCP, discovers the advertised tools, resources and prompts,
and follows bounded list pagination. `mcp_descriptor` adapts tool schemas into
the shared descriptor command/form model. `descriptor_widgets` paints the list
and fields; submission converts values to JSON before admission and transport.
Arrays and objects accept JSON text. Results and errors appear below the form
with a Copy button.

Use j/k or arrow keys to select, Enter to open or submit, Tab to switch list
views or move between form controls, and Esc to return. Enter on a disconnected
list retries the connection. The Connect / retry button does the same.

`service_external` drains responses from the host logic pass, including hidden
windows and inactive contexts. Pending requests have a deadline; a timeout
closes the connection and asks the user to reconnect. Closing the pane drops
the transport, whose process-group cleanup remains owned by `McpConnection`.
The visualizer does not persist a live connection across layout restoration.
