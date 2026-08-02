# Captured MCP server traffic

Every file here is a **real JSON-RPC message captured from a real MCP server**,
stored verbatim in wire form (one compact JSON object, newline-terminated).
Nothing here was written by hand or reconstructed from documentation — a fixture
invented from an assumed response shape produces tests that validate the
assumption rather than the protocol.

| File | Server | Message |
|---|---|---|
| `filesystem_initialize.json` | `npx -y @modelcontextprotocol/server-filesystem /tmp` (`secure-filesystem-server` 0.2.0) | `initialize` result |
| `filesystem_tools_list.json` | same | `tools/list` result |
| `everything_tools_list.json` | `npx -y @modelcontextprotocol/server-everything` (`mcp-servers/everything` 2.0.0) | `tools/list` result |

The `everything` capture is the schema-translation torture case: it carries real
`annotations.readOnlyHint` values, a tool with an `outputSchema`, and tool names
using `-` rather than `.`. The `filesystem` capture carries draft-07 `$schema`
keys and `array`/`number` typed properties.

To recapture, drive a server over stdio with `initialize` →
`notifications/initialized` → `tools/list` and store each result object as-is.
