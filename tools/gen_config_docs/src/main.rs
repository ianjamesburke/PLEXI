//! Generates website/src/content/docs/config.md from serde config structs.
//! Usage: cargo run -p gen_config_docs > website/src/content/docs/config.md

use std::collections::HashMap;
use syn::{Fields, File, Item, ItemStruct, Type};

struct FieldInfo {
    name: String,
    ty: String,
    default: String,
    doc: String,
}

struct StructInfo {
    name: String,
    fields: Vec<FieldInfo>,
}

fn main() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let config_src = std::path::Path::new(manifest_dir).join("../../src/config/mod.rs");
    let default_config =
        std::path::Path::new(manifest_dir).join("../../scripts/default-config.toml");

    let source = std::fs::read_to_string(&config_src)
        .unwrap_or_else(|e| panic!("read {}: {e}", config_src.display()));
    let default_toml = std::fs::read_to_string(&default_config)
        .unwrap_or_else(|e| panic!("read {}: {e}", default_config.display()));

    let file: File = syn::parse_str(&source).expect("parse src/config/mod.rs");

    let structs: HashMap<String, StructInfo> = file
        .items
        .iter()
        .filter_map(|item| {
            if let Item::Struct(s) = item {
                if derives_deserialize(s) {
                    let info = extract_struct(s);
                    Some((info.name.clone(), info))
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    emit_doc(&structs, &default_toml);
}

fn derives_deserialize(s: &ItemStruct) -> bool {
    s.attrs.iter().any(|attr| {
        if attr.path().is_ident("derive") {
            if let syn::Meta::List(list) = &attr.meta {
                list.tokens.to_string().contains("Deserialize")
            } else {
                false
            }
        } else {
            false
        }
    })
}

fn extract_struct(s: &ItemStruct) -> StructInfo {
    let fields = if let Fields::Named(named) = &s.fields {
        named
            .named
            .iter()
            .map(|f| {
                let name = f.ident.as_ref().unwrap().to_string();
                let ty = simplify_type(&f.ty);
                let doc = extract_doc(&f.attrs);
                let default = extract_default(&doc);
                FieldInfo {
                    name,
                    ty,
                    default,
                    doc,
                }
            })
            .collect()
    } else {
        vec![]
    };

    StructInfo {
        name: s.ident.to_string(),
        fields,
    }
}

fn extract_doc(attrs: &[syn::Attribute]) -> String {
    attrs
        .iter()
        .filter_map(|attr| {
            if attr.path().is_ident("doc") {
                if let syn::Meta::NameValue(nv) = &attr.meta {
                    if let syn::Expr::Lit(lit) = &nv.value {
                        if let syn::Lit::Str(s) = &lit.lit {
                            let v = s.value();
                            let trimmed = v.trim().to_string();
                            if !trimmed.is_empty() {
                                return Some(trimmed);
                            }
                        }
                    }
                }
            }
            None
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn simplify_type(ty: &Type) -> String {
    match ty {
        Type::Path(p) => {
            let Some(seg) = p.path.segments.last() else {
                return "—".to_string();
            };
            let name = seg.ident.to_string();
            if name == "Option" {
                if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                    if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                        return simplify_type(inner);
                    }
                }
            }
            match name.as_str() {
                "bool" => "bool".to_string(),
                "String" => "string".to_string(),
                "f32" | "f64" => "float".to_string(),
                "u32" | "u64" | "usize" => "integer".to_string(),
                "HashMap" => "table".to_string(),
                _ => name,
            }
        }
        _ => "—".to_string(),
    }
}

fn extract_default(doc: &str) -> String {
    if doc.is_empty() {
        return "—".to_string();
    }
    let lower = doc.to_lowercase();
    let prefixes = ["default: ", "defaults to ", "; default ", ". default "];
    for prefix in &prefixes {
        if let Some(pos) = lower.find(prefix) {
            let after = &doc[pos + prefix.len()..];
            let val = cut_value(after);
            if !val.is_empty() {
                return val;
            }
        }
    }
    // "(default: X)"
    const PAREN_DEFAULT: &str = "(default: ";
    if let Some(pos) = lower.find(PAREN_DEFAULT) {
        let after = &doc[pos + PAREN_DEFAULT.len()..];
        let val = cut_value(after);
        if !val.is_empty() {
            return val;
        }
    }
    "—".to_string()
}

fn cut_value(after: &str) -> String {
    // Stop at the first sentence-ending or clause-starting punctuation.
    let mut end = after.len();
    for stop in [" (", " —"] {
        if let Some(p) = after.find(stop) {
            end = end.min(p);
        }
    }
    for c in [';', ')', '\n'] {
        if let Some(p) = after.find(c) {
            end = end.min(p);
        }
    }
    // Stop at '.' only when NOT a decimal point (digit follows).
    let bytes = after.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if i >= end {
            break;
        }
        if b == b'.' && !matches!(bytes.get(i + 1), Some(b'0'..=b'9')) {
            end = end.min(i);
            break;
        }
    }
    after[..end].trim().to_string()
}

fn emit_table(fields: &[FieldInfo]) {
    println!("| Field | Type | Default | Description |");
    println!("|---|---|---|---|");
    for f in fields {
        let doc = if f.doc.is_empty() { "—" } else { &f.doc };
        println!("| `{}` | {} | {} | {} |", f.name, f.ty, f.default, doc);
    }
    println!();
}

fn emit_doc(structs: &HashMap<String, StructInfo>, default_toml: &str) {
    // ── Frontmatter ─────────────────────────────────────────────────────────
    print!(
        r#"---
title: Configuration
description: Edit config.toml, choose a theme, wire AI models, and override shortcuts.
order: 2
---

Plexi reads `config.toml` from the active channel profile. Alpha uses `~/.plexi-alpha/config.toml`, beta uses `~/.plexi-beta/config.toml`, main uses `~/.plexi/config.toml`, and PR builds use `~/.plexi-pr-<N>/config.toml`.

Changes take effect on the next launch unless a command says otherwise.

## Channel discipline

Alpha config stays default. `just install` refreshes `~/.plexi-alpha/config.toml` from the built-in template, and PR builds seed from alpha. Do not use alpha for personal overrides you expect to keep.

Beta config is the staging ground. `~/.plexi-beta/config.toml` is not reset by alpha installs, so it is the right place to test migrations, advanced settings, and personal config before promoting a release.

PR builds are isolated. A PR build reads `~/.plexi-pr-<N>/config.toml`; use the `plexi-pr-<N>` binary when checking a PR-specific config behavior.

## Commands

```sh
plexi config edit
plexi config check
plexi config reset
```

`config edit` opens the active channel config. `config check` validates known keys and TOML syntax. `config reset` backs up the current file to `config.toml.bak`, then writes the built-in default template.

## Reference

### Top-level (`config.toml`)

"#
    );

    // Top-level scalar fields (skip subsection keys)
    const SKIP_TOPLEVEL: &[&str] = &[
        "theme",
        "effects",
        "log",
        "notifications",
        "ai",
        "keybindings",
        "agents",
        "cli",
        "marketplace",
        "file_handlers",
    ];
    if let Some(s) = structs.get("PlexiConfig") {
        let scalar: Vec<&FieldInfo> = s
            .fields
            .iter()
            .filter(|f| !SKIP_TOPLEVEL.contains(&f.name.as_str()))
            .collect();
        emit_table_refs(&scalar);
    }

    // ── Theme ────────────────────────────────────────────────────────────────
    print!(
        r#"### Theme (`[theme]`)

Pick a preset and optionally override individual colors. Individual color fields override preset values when both are set.

Available presets: `catppuccin-mocha`, `catppuccin-latte`, `dracula`, `tokyo-night`, `tokyo-day`, `gruvbox-dark`, `gruvbox-light`, `nord`, `solarized-dark`, `solarized-light`, `matrix`, `bios`, `plexi-night`, `plexi-day`.

"#
    );
    if let Some(s) = structs.get("ThemeConfig") {
        emit_table(&s.fields);
    }

    // ── Effects ──────────────────────────────────────────────────────────────
    println!("### Effects (`[effects]`)\n");
    if let Some(s) = structs.get("EffectsConfig") {
        emit_table(&s.fields);
    }

    // ── Log ──────────────────────────────────────────────────────────────────
    print!(
        r#"### Log (`[log]`)

`level` accepts `error`, `warn`, `info`, or `debug`. Applies live on config save — no restart required.

"#
    );
    if let Some(s) = structs.get("LogConfig") {
        emit_table(&s.fields);
    }

    // ── Notifications ────────────────────────────────────────────────────────
    print!(
        r#"### Notifications (`[notifications]`)

Visible notifications open the modal unless `focus_mode` is enabled. There is one notification level.

"#
    );
    if let Some(s) = structs.get("NotificationsConfig") {
        emit_table(&s.fields);
    }

    // ── AI ───────────────────────────────────────────────────────────────────
    print!(
        r#"### AI (`[ai]`)

API keys are NOT stored in `config.toml`. Export `OPENROUTER_API_KEY` in your shell profile (`~/.zshrc`, `~/.zprofile`, etc.), or store it in the Plexi secret store:

```sh
plexi secret set openrouter-api-key --global
```

"#
    );
    if let Some(s) = structs.get("AiConfig") {
        let scalar: Vec<&FieldInfo> = s
            .fields
            .iter()
            .filter(|f| f.name != "openrouter" && f.name != "ollama" && f.name != "local")
            .collect();
        emit_table_refs(&scalar);
    }

    println!("#### OpenRouter (`[ai.openrouter]`)\n");
    if let Some(s) = structs.get("OpenRouterBackendConfig") {
        emit_table(&s.fields);
    }

    println!("#### Ollama (`[ai.ollama]`)\n");
    if let Some(s) = structs.get("OllamaBackendConfig") {
        emit_table(&s.fields);
    }

    println!("#### Local OpenAI-compatible (`[ai.local]`)\n");
    if let Some(s) = structs.get("LocalBackendConfig") {
        emit_table(&s.fields);
    }

    // ── Agents ───────────────────────────────────────────────────────────────
    print!(
        r#"### Agents (`[agents]`)

Command templates for dispatching coding agents. `{{cmd}}` is replaced with the prompt or slash command at dispatch time.

"#
    );
    if let Some(s) = structs.get("AgentsConfig") {
        emit_table(&s.fields);
    }

    // ── Keybindings ──────────────────────────────────────────────────────────
    print!(
        r#"### Keybindings (`[keybindings]`)

Override only the shortcuts you want to change. Unknown or conflicting overrides log a warning at startup and keep the default binding.

Modifier keys: `cmd` (alias: `command`), `shift`, `ctrl` (alias: `control`), `alt` (aliases: `opt`, `option`).

Key names: `a`–`z`, `0`–`9`, `enter`, `escape`, `tab`, `space`, `backspace`, `delete`, `up`, `down`, `left`, `right`, `open_bracket`, `close_bracket`, `backslash`, `slash`, `comma`, `period`, `equals`, `minus`.

Each value is a string like `"cmd+p"` or `"cmd+shift+w"`.

"#
    );
    if let Some(s) = structs.get("KeybindingsConfig") {
        println!("| Action | Description |");
        println!("|---|---|");
        for f in &s.fields {
            let desc = if f.doc.is_empty() {
                human_action_name(&f.name)
            } else {
                f.doc.clone()
            };
            println!("| `{}` | {} |", f.name, desc);
        }
        println!();
    }

    // ── CLI ──────────────────────────────────────────────────────────────────
    println!("### CLI (`[cli]`)\n");
    if let Some(s) = structs.get("CliConfig") {
        emit_table(&s.fields);
    }

    // ── File handlers ────────────────────────────────────────────────────────
    print!(
        r#"### File handlers (`[file_handlers]`)

Choose which app opens each file type from the File Explorer. Keys are bare lowercase extensions (e.g. `md`, not `.md`). Values are handler specs:

| Value | Behavior |
|---|---|
| `app:<id>` | Open in a Plexi app by its registry id (e.g. `app:text-editor`) |
| `os` | Hand to the OS default opener (`open` / `xdg-open`) |
| `cmd:<command>` | Reserved; falls back to `os` until wired |

A handler here overrides an app's own `file_types` association. Unmapped extensions fall through to manifest associations, then builtin media players, then the OS default.

"#
    );

    // ── Marketplace ──────────────────────────────────────────────────────────
    print!(
        r#"### Marketplace (`[marketplace]`)

All fields are optional. Omitting the section uses the official `plexiapp.com` registry and CDN. Override only to point at a private registry or to test publishing flows.

"#
    );
    if let Some(s) = structs.get("MarketplaceConfig") {
        emit_table(&s.fields);
    }

    // ── Default config.toml ──────────────────────────────────────────────────
    println!("## Default config.toml\n");
    println!(
        "This is the built-in template Plexi writes when it creates or resets the active channel config.\n"
    );
    println!("```toml");
    print!("{default_toml}");
    if !default_toml.ends_with('\n') {
        println!();
    }
    println!("```");
}

fn emit_table_refs(fields: &[&FieldInfo]) {
    println!("| Field | Type | Default | Description |");
    println!("|---|---|---|---|");
    for f in fields {
        let doc = if f.doc.is_empty() { "—" } else { &f.doc };
        println!("| `{}` | {} | {} | {} |", f.name, f.ty, f.default, doc);
    }
    println!();
}

fn human_action_name(action: &str) -> String {
    action.replace('_', " ")
}
