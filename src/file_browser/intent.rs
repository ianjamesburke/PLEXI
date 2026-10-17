use std::path::{Path, PathBuf};

use super::helpers::{ColumnId, Entry, SortDirection, default_direction_for};

/// Typed intent contract for the File Explorer `/` and `Cmd+K` action palette.
///
/// Both explicit slash commands (`/hidden`, `/new file notes.md`, `/sort size asc`)
/// and natural-language / JEV phrases (`"show hidden files"`, `"create file foo.rs"`,
/// `"rename to main_v2.rs"`) resolve to this single serializable enum before
/// mutating explorer state or dispatching a `FileBrowserAction`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FileExplorerIntent {
    CreateFile {
        name: String,
        open_in_editor: bool,
    },
    CreateFolder {
        name: String,
    },
    Rename {
        target: PathBuf,
        new_name: String,
    },
    MoveToTrash {
        targets: Vec<PathBuf>,
    },
    OpenInEditor {
        path: PathBuf,
    },
    OpenWithDefault {
        path: PathBuf,
    },
    Reveal {
        path: PathBuf,
    },
    CdTerminal {
        path: PathBuf,
    },
    SortBy {
        column: ColumnId,
        direction: SortDirection,
    },
    ToggleHidden {
        enabled: Option<bool>,
    },
    ToggleFoldersFirst {
        enabled: Option<bool>,
    },
}

/// Live File Explorer context used to resolve relative paths and selection-bound
/// intents (`Rename`, `MoveToTrash`, `OpenInEditor`, `Reveal`, `CdTerminal`).
#[derive(Debug, Clone)]
pub struct FileExplorerContext<'a> {
    pub cwd: &'a Path,
    pub selected_entry: Option<&'a Entry>,
    pub selected_paths: &'a [PathBuf],
    pub entries: &'a [Entry],
    pub show_hidden: bool,
    pub folders_on_top: bool,
}

impl<'a> FileExplorerContext<'a> {
    pub fn new(
        cwd: &'a Path,
        selected_entry: Option<&'a Entry>,
        selected_paths: &'a [PathBuf],
        entries: &'a [Entry],
        show_hidden: bool,
        folders_on_top: bool,
    ) -> Self {
        Self {
            cwd,
            selected_entry,
            selected_paths,
            entries,
            show_hidden,
            folders_on_top,
        }
    }

    fn primary_target_path(&self) -> Option<PathBuf> {
        self.selected_entry
            .map(|entry| entry.path.clone())
            .or_else(|| self.selected_paths.first().cloned())
    }

    fn trash_target_paths(&self) -> Vec<PathBuf> {
        if !self.selected_paths.is_empty() {
            self.selected_paths.to_vec()
        } else if let Some(entry) = self.selected_entry {
            vec![entry.path.clone()]
        } else {
            Vec::new()
        }
    }

    fn cd_target_path(&self) -> PathBuf {
        if let Some(entry) = self.selected_entry {
            if entry.is_dir {
                return entry.path.clone();
            }
        }
        self.cwd.to_path_buf()
    }

    fn resolve_named_target(&self, raw: &str) -> Option<PathBuf> {
        let trimmed = strip_surrounding_quotes(raw.trim());
        if trimmed.is_empty() {
            return None;
        }
        if is_selection_pronoun(trimmed) {
            return self.primary_target_path();
        }
        if matches!(trimmed, "." | "here" | "cwd" | "current directory" | "current folder") {
            return Some(self.cwd.to_path_buf());
        }
        if let Some(entry) = self
            .entries
            .iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(trimmed))
        {
            return Some(entry.path.clone());
        }
        let candidate = Path::new(trimmed);
        if candidate.is_absolute() {
            Some(candidate.to_path_buf())
        } else {
            Some(self.cwd.join(trimmed))
        }
    }
}

/// Resolution state for a candidate row in the action palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateResolution {
    /// Complete intent ready to execute on `Enter`.
    Ready(FileExplorerIntent),
    /// Command template waiting for a required slot argument; pressing `Enter`
    /// or `Tab` pre-fills `prompt_prefix` in the input field.
    NeedsSlot {
        prompt_prefix: &'static str,
        slot_hint: &'static str,
    },
}

/// A ranked row in the File Explorer `/` or `Cmd+K` palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteCandidate {
    pub slash_syntax: &'static str,
    pub title: String,
    pub subtitle: String,
    pub preview_chip: String,
    pub resolution: CandidateResolution,
    pub score: i32,
}

impl FileExplorerIntent {
    /// Human-readable structured preview chip summarizing the resolved action
    /// and its filled-in slots before the user presses `Enter`.
    pub fn preview_chip(&self, ctx: &FileExplorerContext<'_>) -> String {
        match self {
            Self::CreateFile {
                name,
                open_in_editor,
            } => {
                if *open_in_editor {
                    format!("Create file: {name} — open in editor [Enter]")
                } else {
                    format!("Create file: {name} [Enter]")
                }
            }
            Self::CreateFolder { name } => format!("Create folder: {name} [Enter]"),
            Self::Rename { target, new_name } => {
                let from = target
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("item");
                format!("Rename: {from} → {new_name} [Enter]")
            }
            Self::MoveToTrash { targets } => {
                if targets.len() == 1 {
                    let name = targets[0]
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("selection");
                    format!("Move to Trash: {name} [Enter]")
                } else {
                    format!("Move to Trash: {} items [Enter]", targets.len())
                }
            }
            Self::OpenInEditor { path } => {
                let name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("selection");
                format!("Open in editor: {name} [Enter]")
            }
            Self::OpenWithDefault { path } => {
                let name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("selection");
                format!("Open with default app: {name} [Enter]")
            }
            Self::Reveal { path } => {
                let name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("folder");
                format!("Reveal in system file manager: {name} [Enter]")
            }
            Self::CdTerminal { path } => {
                let name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("current folder");
                format!("cd linked terminal: {name} [Enter]")
            }
            Self::SortBy { column, direction } => {
                let dir_label = match direction {
                    SortDirection::Asc => "Asc",
                    SortDirection::Desc => "Desc",
                };
                format!("Sort by: {} ({dir_label}) [Enter]", column.label())
            }
            Self::ToggleHidden { enabled } => {
                let next = enabled.unwrap_or(!ctx.show_hidden);
                let state_str = if next { "ON" } else { "OFF" };
                format!("Toggle hidden files: {state_str} [Enter]")
            }
            Self::ToggleFoldersFirst { enabled } => {
                let next = enabled.unwrap_or(!ctx.folders_on_top);
                let state_str = if next { "ON" } else { "OFF" };
                format!("Folders first: {state_str} [Enter]")
            }
        }
    }
}

/// Parse an explicit slash command or natural-language / JEV phrase into a
/// typed `FileExplorerIntent` against the current `FileExplorerContext`.
pub fn parse_file_explorer_intent(
    input: &str,
    ctx: &FileExplorerContext<'_>,
) -> Option<FileExplorerIntent> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return None;
    }

    if let Some(after_slash) = trimmed.strip_prefix('/') {
        let slash_body = after_slash.trim_start_matches('/').trim();
        if slash_body.is_empty() {
            return None;
        }
        if let Some(intent) = parse_slash_intent(slash_body, ctx) {
            return Some(intent);
        }
        // Allow natural-language phrases typed after a leading `/` as well.
        return parse_natural_language_intent(slash_body, ctx);
    }

    parse_natural_language_intent(trimmed, ctx)
}

fn parse_slash_intent(body: &str, ctx: &FileExplorerContext<'_>) -> Option<FileExplorerIntent> {
    let lower = body.to_ascii_lowercase();
    let (head, rest) = split_first_word(body);
    let head_lower = head.to_ascii_lowercase();

    match head_lower.as_str() {
        "hidden" | "dotfiles" | "show-hidden" | "hide-hidden" => {
            let enabled = if head_lower == "show-hidden" {
                Some(true)
            } else if head_lower == "hide-hidden" {
                Some(false)
            } else {
                parse_optional_bool_flag(rest)
            };
            Some(FileExplorerIntent::ToggleHidden { enabled })
        }
        "folders-first" | "folders" | "folders-on-top" => {
            let enabled = parse_optional_bool_flag(rest);
            Some(FileExplorerIntent::ToggleFoldersFirst { enabled })
        }
        "new-file" | "touch" => {
            let (name, open_in_editor) = parse_file_name_and_open_flag(rest)?;
            Some(FileExplorerIntent::CreateFile {
                name,
                open_in_editor,
            })
        }
        "new-folder" | "mkdir" => {
            let name = sanitize_single_name(rest)?;
            Some(FileExplorerIntent::CreateFolder { name })
        }
        "new" | "create" | "make" | "add" => {
            let (kind, tail) = split_first_word(rest);
            match kind.to_ascii_lowercase().as_str() {
                "file" => {
                    let (name, open_in_editor) = parse_file_name_and_open_flag(tail)?;
                    Some(FileExplorerIntent::CreateFile {
                        name,
                        open_in_editor,
                    })
                }
                "folder" | "dir" | "directory" => {
                    let name = sanitize_single_name(tail)?;
                    Some(FileExplorerIntent::CreateFolder { name })
                }
                _ => None,
            }
        }
        "rename" | "mv" => parse_rename_clause(rest, ctx),
        "trash" | "delete" | "rm" | "remove" => {
            if rest.is_empty() || is_selection_pronoun(rest) {
                let targets = ctx.trash_target_paths();
                if targets.is_empty() {
                    None
                } else {
                    Some(FileExplorerIntent::MoveToTrash { targets })
                }
            } else {
                let target = ctx.resolve_named_target(rest)?;
                Some(FileExplorerIntent::MoveToTrash {
                    targets: vec![target],
                })
            }
        }
        "open" | "edit" => {
            let path = if rest.is_empty() {
                ctx.primary_target_path()?
            } else {
                ctx.resolve_named_target(rest)?
            };
            Some(FileExplorerIntent::OpenInEditor { path })
        }
        "open-default" | "default" | "system-open" => {
            let path = if rest.is_empty() {
                ctx.primary_target_path()?
            } else {
                ctx.resolve_named_target(rest)?
            };
            Some(FileExplorerIntent::OpenWithDefault { path })
        }
        "reveal" | "finder" => {
            let path = if rest.is_empty() {
                ctx.primary_target_path()
                    .unwrap_or_else(|| ctx.cwd.to_path_buf())
            } else {
                ctx.resolve_named_target(rest)?
            };
            Some(FileExplorerIntent::Reveal { path })
        }
        "cd" | "terminal" => {
            let path = if rest.is_empty() {
                ctx.cd_target_path()
            } else {
                ctx.resolve_named_target(rest)?
            };
            Some(FileExplorerIntent::CdTerminal { path })
        }
        "sort" | "order" => parse_sort_clause(rest),
        _ => {
            // Support `/sort-size`, `/sort-name`, etc.
            if let Some(col_part) = lower.strip_prefix("sort-") {
                return parse_sort_clause(col_part);
            }
            None
        }
    }
}

fn parse_natural_language_intent(
    text: &str,
    ctx: &FileExplorerContext<'_>,
) -> Option<FileExplorerIntent> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();

    // 1. Hidden / dotfiles intents
    if let Some(intent) = parse_hidden_nl(&lower) {
        return Some(intent);
    }

    // 2. Folders-first intents
    if let Some(intent) = parse_folders_first_nl(&lower) {
        return Some(intent);
    }

    // 3. Sort intents
    if let Some(intent) = parse_sort_nl(&lower) {
        return Some(intent);
    }

    // 4. Create folder vs create file intents
    if let Some(intent) = parse_create_folder_nl(trimmed) {
        return Some(intent);
    }
    if let Some(intent) = parse_create_file_nl(trimmed) {
        return Some(intent);
    }

    // 5. Rename intents
    if let Some(intent) = parse_rename_nl(trimmed, ctx) {
        return Some(intent);
    }

    // 6. Trash / delete intents
    if let Some(intent) = parse_trash_nl(trimmed, ctx) {
        return Some(intent);
    }

    // 7. Reveal / cd / open default / open in editor intents
    if let Some(intent) = parse_navigation_or_open_nl(trimmed, ctx) {
        return Some(intent);
    }

    None
}

fn parse_hidden_nl(lower: &str) -> Option<FileExplorerIntent> {
    let mentions_hidden = lower.contains("hidden") || lower.contains("dotfile");
    if !mentions_hidden {
        return None;
    }
    if lower.starts_with("show ")
        || lower.starts_with("enable ")
        || lower.starts_with("unhide ")
        || lower.starts_with("reveal ")
        || lower.ends_with(" on")
    {
        return Some(FileExplorerIntent::ToggleHidden {
            enabled: Some(true),
        });
    }
    if lower.starts_with("hide ")
        || lower.starts_with("disable ")
        || lower.starts_with("don't show ")
        || lower.ends_with(" off")
    {
        return Some(FileExplorerIntent::ToggleHidden {
            enabled: Some(false),
        });
    }
    if matches!(
        lower,
        "hidden"
            | "hidden files"
            | "dotfiles"
            | "toggle hidden"
            | "toggle hidden files"
            | "toggle dotfiles"
    ) {
        return Some(FileExplorerIntent::ToggleHidden { enabled: None });
    }
    None
}

fn parse_folders_first_nl(lower: &str) -> Option<FileExplorerIntent> {
    let mentions_folders_first = lower.contains("folders first")
        || lower.contains("folders on top")
        || lower.contains("directories first");
    if !mentions_folders_first {
        return None;
    }
    if lower.starts_with("disable ")
        || lower.starts_with("don't ")
        || lower.starts_with("do not ")
        || lower.ends_with(" off")
        || lower.starts_with("unpin ")
    {
        return Some(FileExplorerIntent::ToggleFoldersFirst {
            enabled: Some(false),
        });
    }
    if lower.starts_with("put ")
        || lower.starts_with("show ")
        || lower.starts_with("keep ")
        || lower.starts_with("enable ")
        || lower.starts_with("pin ")
        || lower.ends_with(" on")
    {
        return Some(FileExplorerIntent::ToggleFoldersFirst {
            enabled: Some(true),
        });
    }
    Some(FileExplorerIntent::ToggleFoldersFirst { enabled: None })
}

fn parse_sort_nl(lower: &str) -> Option<FileExplorerIntent> {
    let rest = lower
        .strip_prefix("sort by ")
        .or_else(|| lower.strip_prefix("sort "))
        .or_else(|| lower.strip_prefix("order by "))?;
    parse_sort_clause(rest)
}

fn parse_sort_clause(clause: &str) -> Option<FileExplorerIntent> {
    let cleaned = clause.trim().strip_prefix("by ").unwrap_or(clause.trim());
    if cleaned.is_empty() {
        return None;
    }
    let tokens: Vec<String> = cleaned
        .split_whitespace()
        .map(|tok| tok.trim_matches(|c: char| c == ',' || c == ';').to_ascii_lowercase())
        .filter(|tok| !tok.is_empty())
        .collect();
    if tokens.is_empty() {
        return None;
    }

    // Handle single-word superlative shorthands like "largest", "smallest", "newest", "oldest".
    let mut column: Option<ColumnId> = None;
    let mut direction: Option<SortDirection> = None;

    for token in &tokens {
        match token.as_str() {
            "name" | "names" | "filename" | "alphabetical" | "alpha" => {
                column = Some(ColumnId::Name);
            }
            "kind" | "type" => {
                column = Some(ColumnId::Kind);
            }
            "size" | "bytes" => {
                column = Some(ColumnId::Size);
            }
            "largest" | "biggest" | "heaviest" => {
                column = Some(ColumnId::Size);
                direction = Some(SortDirection::Desc);
            }
            "smallest" | "shortest" => {
                column = Some(ColumnId::Size);
                direction = Some(SortDirection::Asc);
            }
            "modified" | "mtime" | "date" | "time" | "updated" | "recent" => {
                column = Some(ColumnId::Modified);
            }
            "newest" | "latest" => {
                column = Some(ColumnId::Modified);
                direction = Some(SortDirection::Desc);
            }
            "oldest" => {
                column = Some(ColumnId::Modified);
                direction = Some(SortDirection::Asc);
            }
            "created" | "birthtime" | "added" => {
                column = Some(ColumnId::Created);
            }
            "ext" | "extension" => {
                column = Some(ColumnId::Extension);
            }
            "perm" | "perms" | "permissions" | "mode" => {
                column = Some(ColumnId::Permissions);
            }
            "asc" | "ascending" | "up" | "a-z" => {
                direction = Some(SortDirection::Asc);
            }
            "desc" | "descending" | "down" | "z-a" => {
                direction = Some(SortDirection::Desc);
            }
            _ => {}
        }
    }

    let col = column?;
    let dir = direction.unwrap_or_else(|| default_direction_for(col));
    Some(FileExplorerIntent::SortBy {
        column: col,
        direction: dir,
    })
}

fn parse_create_folder_nl(text: &str) -> Option<FileExplorerIntent> {
    let lower = text.to_ascii_lowercase();
    if let Some(rest) = strip_prefix_case_insensitive(text, "mkdir ") {
        let name = sanitize_single_name(rest)?;
        return Some(FileExplorerIntent::CreateFolder { name });
    }

    let starts_with_create = ["create ", "make ", "new ", "add "]
        .iter()
        .any(|p| lower.starts_with(p));
    let mentions_folder =
        lower.contains("folder") || lower.contains("directory") || lower.contains(" dir ");
    if !starts_with_create || !mentions_folder {
        return None;
    }

    let mut cursor = text.trim();
    for prefix in [
        "create", "make", "add", "new", "a", "an", "new", "folder", "directory", "dir", "called",
        "named",
    ] {
        if let Some(after) = strip_word_prefix_case_insensitive(cursor, prefix) {
            cursor = after.trim();
        }
    }
    // Second pass in case article/noun order was e.g. "make a new folder called assets".
    for prefix in [
        "a", "an", "new", "folder", "directory", "dir", "called", "named",
    ] {
        if let Some(after) = strip_word_prefix_case_insensitive(cursor, prefix) {
            cursor = after.trim();
        }
    }

    let name = sanitize_single_name(cursor)?;
    Some(FileExplorerIntent::CreateFolder { name })
}

fn parse_create_file_nl(text: &str) -> Option<FileExplorerIntent> {
    let lower = text.to_ascii_lowercase();
    if let Some(rest) = strip_prefix_case_insensitive(text, "touch ") {
        let (name, open_in_editor) = parse_file_name_and_open_flag(rest)?;
        return Some(FileExplorerIntent::CreateFile {
            name,
            open_in_editor,
        });
    }

    let starts_with_create = ["create ", "make ", "new ", "add "]
        .iter()
        .any(|p| lower.starts_with(p));
    if !starts_with_create {
        return None;
    }
    if lower.contains("folder") || lower.contains("directory") {
        return None;
    }

    let mut cursor = text.trim();
    for prefix in [
        "create", "make", "add", "new", "a", "an", "new", "file", "called", "named",
    ] {
        if let Some(after) = strip_word_prefix_case_insensitive(cursor, prefix) {
            cursor = after.trim();
        }
    }
    for prefix in ["a", "an", "new", "file", "called", "named"] {
        if let Some(after) = strip_word_prefix_case_insensitive(cursor, prefix) {
            cursor = after.trim();
        }
    }

    let (name, open_in_editor) = parse_file_name_and_open_flag(cursor)?;
    // If the user didn't explicitly write the word "file", require the token to look like a filename
    // (contains a `.` or matches known extensionless filenames) so we don't misclassify unrelated phrases.
    if !lower.contains("file") && !looks_like_filename(&name) {
        return None;
    }
    Some(FileExplorerIntent::CreateFile {
        name,
        open_in_editor,
    })
}

fn parse_rename_nl(text: &str, ctx: &FileExplorerContext<'_>) -> Option<FileExplorerIntent> {
    let rest = strip_word_prefix_case_insensitive(text.trim(), "rename")?;
    parse_rename_clause(rest, ctx)
}

fn parse_rename_clause(clause: &str, ctx: &FileExplorerContext<'_>) -> Option<FileExplorerIntent> {
    let trimmed = clause.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Pattern 1: "to <new_name>" (targets current selection)
    if let Some(after_to) = strip_word_prefix_case_insensitive(trimmed, "to") {
        let new_name = sanitize_single_name(after_to)?;
        let target = ctx.primary_target_path()?;
        return Some(FileExplorerIntent::Rename { target, new_name });
    }

    // Pattern 2: "<target> to <new_name>" or "<target> -> <new_name>"
    if let Some((lhs, rhs)) = split_case_insensitive_delimiter(trimmed, " to ")
        .or_else(|| split_case_insensitive_delimiter(trimmed, " -> "))
    {
        let new_name = sanitize_single_name(rhs)?;
        let target = ctx.resolve_named_target(lhs)?;
        return Some(FileExplorerIntent::Rename { target, new_name });
    }

    // Pattern 3: bare "<new_name>" with an active selection
    let new_name = sanitize_single_name(trimmed)?;
    let target = ctx.primary_target_path()?;
    Some(FileExplorerIntent::Rename { target, new_name })
}

fn parse_trash_nl(text: &str, ctx: &FileExplorerContext<'_>) -> Option<FileExplorerIntent> {
    let lower = text.trim().to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "trash"
            | "trash this"
            | "trash selected"
            | "trash selection"
            | "move to trash"
            | "move this to trash"
            | "move selected to trash"
            | "delete"
            | "delete this"
            | "delete selected"
            | "remove this"
            | "remove selected"
    ) {
        let targets = ctx.trash_target_paths();
        return if targets.is_empty() {
            None
        } else {
            Some(FileExplorerIntent::MoveToTrash { targets })
        };
    }

    if let Some(after_move) = strip_word_prefix_case_insensitive(text.trim(), "move") {
        if let Some((target_part, _)) = split_case_insensitive_delimiter(after_move, " to trash") {
            let target = ctx.resolve_named_target(target_part)?;
            return Some(FileExplorerIntent::MoveToTrash {
                targets: vec![target],
            });
        }
    }

    for verb in ["trash", "delete", "remove"] {
        if let Some(rest) = strip_word_prefix_case_insensitive(text.trim(), verb) {
            let target = ctx.resolve_named_target(rest)?;
            return Some(FileExplorerIntent::MoveToTrash {
                targets: vec![target],
            });
        }
    }

    None
}

fn parse_navigation_or_open_nl(
    text: &str,
    ctx: &FileExplorerContext<'_>,
) -> Option<FileExplorerIntent> {
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();

    // Reveal in Finder / system file manager
    if matches!(
        lower.as_str(),
        "reveal" | "reveal this" | "reveal in finder" | "show in finder" | "open in finder"
    ) {
        let path = ctx
            .primary_target_path()
            .unwrap_or_else(|| ctx.cwd.to_path_buf());
        return Some(FileExplorerIntent::Reveal { path });
    }
    if let Some(rest) = strip_word_prefix_case_insensitive(trimmed, "reveal") {
        let cleaned = strip_suffix_case_insensitive(rest.trim(), " in finder").unwrap_or(rest);
        let path = ctx.resolve_named_target(cleaned)?;
        return Some(FileExplorerIntent::Reveal { path });
    }

    // cd terminal
    if matches!(
        lower.as_str(),
        "cd" | "cd here" | "cd terminal" | "open in terminal" | "cd in terminal"
    ) {
        return Some(FileExplorerIntent::CdTerminal {
            path: ctx.cd_target_path(),
        });
    }
    if let Some(rest) = strip_word_prefix_case_insensitive(trimmed, "cd") {
        let cleaned = strip_word_prefix_case_insensitive(rest.trim(), "to").unwrap_or(rest.trim());
        let path = ctx.resolve_named_target(cleaned)?;
        return Some(FileExplorerIntent::CdTerminal { path });
    }

    // Open with default app
    if matches!(
        lower.as_str(),
        "open with default"
            | "open default"
            | "open in default app"
            | "open with default app"
            | "open with system default"
    ) {
        let path = ctx.primary_target_path()?;
        return Some(FileExplorerIntent::OpenWithDefault { path });
    }

    // Open in editor
    if matches!(
        lower.as_str(),
        "open" | "open this" | "open in editor" | "edit" | "edit this" | "edit file"
    ) {
        let path = ctx.primary_target_path()?;
        return Some(FileExplorerIntent::OpenInEditor { path });
    }
    for verb in ["open", "edit"] {
        if let Some(rest) = strip_word_prefix_case_insensitive(trimmed, verb) {
            let cleaned =
                strip_suffix_case_insensitive(rest.trim(), " in editor").unwrap_or(rest.trim());
            let path = ctx.resolve_named_target(cleaned)?;
            return Some(FileExplorerIntent::OpenInEditor { path });
        }
    }

    None
}

/// Rank palette candidates for the current query and explorer context.
///
/// When `input` resolves directly to a `FileExplorerIntent` (either via slash
/// syntax or natural language), that resolved intent is placed at the top of
/// the candidate list with its structured preview chip so `Enter` executes it
/// immediately.
pub fn rank_file_explorer_candidates(
    input: &str,
    ctx: &FileExplorerContext<'_>,
) -> Vec<PaletteCandidate> {
    let trimmed = input.trim();
    let mut candidates = Vec::new();

    if let Some(resolved) = parse_file_explorer_intent(trimmed, ctx) {
        let chip = resolved.preview_chip(ctx);
        let (slash_syntax, title, subtitle) = describe_resolved_intent(&resolved, ctx);
        candidates.push(PaletteCandidate {
            slash_syntax,
            title,
            subtitle,
            preview_chip: chip,
            resolution: CandidateResolution::Ready(resolved),
            score: 1_000,
        });
    }

    let catalog = build_catalog_candidates(ctx);
    let query_norm = trimmed
        .strip_prefix('/')
        .unwrap_or(trimmed)
        .trim()
        .to_ascii_lowercase();

    for mut item in catalog {
        // Avoid duplicating an identical Ready intent already at index 0.
        if candidates
            .first()
            .is_some_and(|top| top.resolution == item.resolution)
        {
            continue;
        }

        if query_norm.is_empty() {
            candidates.push(item);
            continue;
        }

        if let Some(score) = score_catalog_candidate(&item, &query_norm) {
            item.score = score;
            candidates.push(item);
        }
    }

    candidates.sort_by_key(|b| std::cmp::Reverse(b.score));
    candidates
}

fn describe_resolved_intent(
    intent: &FileExplorerIntent,
    ctx: &FileExplorerContext<'_>,
) -> (&'static str, String, String) {
    match intent {
        FileExplorerIntent::CreateFile {
            name,
            open_in_editor,
        } => (
            "/new file <name>",
            format!("New File: {name}"),
            if *open_in_editor {
                "Create in current folder and open in editor pane".to_string()
            } else {
                "Create in current folder".to_string()
            },
        ),
        FileExplorerIntent::CreateFolder { name } => (
            "/new folder <name>",
            format!("New Folder: {name}"),
            "Create directory in current folder".to_string(),
        ),
        FileExplorerIntent::Rename { target, new_name } => {
            let from = target
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("selection");
            (
                "/rename <new_name>",
                format!("Rename {from} → {new_name}"),
                "Rename item in current folder".to_string(),
            )
        }
        FileExplorerIntent::MoveToTrash { targets } => (
            "/trash",
            if targets.len() == 1 {
                let name = targets[0]
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("selection");
                format!("Move to Trash: {name}")
            } else {
                format!("Move {} items to Trash", targets.len())
            },
            "Move selected entries to system Trash".to_string(),
        ),
        FileExplorerIntent::OpenInEditor { path } => {
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("selection");
            (
                "/open",
                format!("Open in Editor: {name}"),
                "Open file in a Plexi editor/viewer pane".to_string(),
            )
        }
        FileExplorerIntent::OpenWithDefault { path } => {
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("selection");
            (
                "/open-default",
                format!("Open with Default App: {name}"),
                "Launch with the OS default application".to_string(),
            )
        }
        FileExplorerIntent::Reveal { path } => {
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("folder");
            (
                "/reveal",
                format!("Reveal: {name}"),
                "Reveal in system file manager".to_string(),
            )
        }
        FileExplorerIntent::CdTerminal { path } => {
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("current folder");
            (
                "/cd",
                format!("cd Terminal: {name}"),
                "Change directory in linked terminal pane".to_string(),
            )
        }
        FileExplorerIntent::SortBy { column, direction } => (
            "/sort <col> [asc|desc]",
            format!("Sort by {} ({})", column.label(), direction.key()),
            "Update active sort column and direction".to_string(),
        ),
        FileExplorerIntent::ToggleHidden { enabled } => {
            let next = enabled.unwrap_or(!ctx.show_hidden);
            (
                "/hidden",
                format!("Toggle Hidden Files ({})", if next { "ON" } else { "OFF" }),
                "Show or hide dotfiles in File Explorer".to_string(),
            )
        }
        FileExplorerIntent::ToggleFoldersFirst { enabled } => {
            let next = enabled.unwrap_or(!ctx.folders_on_top);
            (
                "/folders-first",
                format!("Pin Folders on Top ({})", if next { "ON" } else { "OFF" }),
                "Keep directories grouped above files".to_string(),
            )
        }
    }
}

fn build_catalog_candidates(ctx: &FileExplorerContext<'_>) -> Vec<PaletteCandidate> {
    let mut list = Vec::new();

    // 1. /new file <name>
    list.push(PaletteCandidate {
        slash_syntax: "/new file <name>",
        title: "New File…".to_string(),
        subtitle: "Create a file in the current directory and open it in the editor".to_string(),
        preview_chip: "Create file: <name> — type filename and press Enter".to_string(),
        resolution: CandidateResolution::NeedsSlot {
            prompt_prefix: "/new file ",
            slot_hint: "<filename>",
        },
        score: 95,
    });

    // 2. /new folder <name>
    list.push(PaletteCandidate {
        slash_syntax: "/new folder <name>",
        title: "New Folder…".to_string(),
        subtitle: "Create a directory in the current folder (Cmd+N)".to_string(),
        preview_chip: "Create folder: <name> — type folder name and press Enter".to_string(),
        resolution: CandidateResolution::NeedsSlot {
            prompt_prefix: "/new folder ",
            slot_hint: "<folder-name>",
        },
        score: 94,
    });

    // 3. /hidden
    let hidden_intent = FileExplorerIntent::ToggleHidden { enabled: None };
    list.push(PaletteCandidate {
        slash_syntax: "/hidden",
        title: format!(
            "Toggle Hidden Files (currently {})",
            if ctx.show_hidden { "ON" } else { "OFF" }
        ),
        subtitle: "Show or hide dotfiles (/dotfiles, Cmd+Shift+.)".to_string(),
        preview_chip: hidden_intent.preview_chip(ctx),
        resolution: CandidateResolution::Ready(hidden_intent),
        score: 92,
    });

    // 4. /folders-first
    let folders_intent = FileExplorerIntent::ToggleFoldersFirst { enabled: None };
    list.push(PaletteCandidate {
        slash_syntax: "/folders-first",
        title: format!(
            "Toggle Folders First (currently {})",
            if ctx.folders_on_top { "ON" } else { "OFF" }
        ),
        subtitle: "Pin folders above files when sorting".to_string(),
        preview_chip: folders_intent.preview_chip(ctx),
        resolution: CandidateResolution::Ready(folders_intent),
        score: 90,
    });

    // 5. /rename <new_name> (when selection exists)
    if let Some(selected) = ctx.selected_entry {
        list.push(PaletteCandidate {
            slash_syntax: "/rename <new_name>",
            title: format!("Rename \"{}\"…", selected.name),
            subtitle: "Rename selected entry inline".to_string(),
            preview_chip: format!(
                "Rename: {} → <new_name> — type new name and press Enter",
                selected.name
            ),
            resolution: CandidateResolution::NeedsSlot {
                prompt_prefix: "/rename ",
                slot_hint: "<new-name>",
            },
            score: 88,
        });
    }

    // 6. /sort variants
    for (col, dir, score) in [
        (ColumnId::Name, SortDirection::Asc, 85),
        (ColumnId::Modified, SortDirection::Desc, 84),
        (ColumnId::Size, SortDirection::Desc, 83),
        (ColumnId::Size, SortDirection::Asc, 82),
        (ColumnId::Kind, SortDirection::Asc, 80),
        (ColumnId::Extension, SortDirection::Asc, 79),
    ] {
        let intent = FileExplorerIntent::SortBy {
            column: col,
            direction: dir,
        };
        list.push(PaletteCandidate {
            slash_syntax: "/sort <col> [asc|desc]",
            title: format!("Sort by {} ({})", col.label(), dir.key()),
            subtitle: format!("/sort {} {}", col.key(), dir.key()),
            preview_chip: intent.preview_chip(ctx),
            resolution: CandidateResolution::Ready(intent),
            score,
        });
    }

    // 7. Selection-bound actions (/open, /open-default, /trash)
    if let Some(path) = ctx.primary_target_path() {
        let open_intent = FileExplorerIntent::OpenInEditor { path: path.clone() };
        list.push(PaletteCandidate {
            slash_syntax: "/open",
            title: "Open Selection in Editor".to_string(),
            subtitle: "Open selected entry in a Plexi pane".to_string(),
            preview_chip: open_intent.preview_chip(ctx),
            resolution: CandidateResolution::Ready(open_intent),
            score: 78,
        });

        let default_intent = FileExplorerIntent::OpenWithDefault { path };
        list.push(PaletteCandidate {
            slash_syntax: "/open-default",
            title: "Open Selection with Default App".to_string(),
            subtitle: "Launch with system default opener".to_string(),
            preview_chip: default_intent.preview_chip(ctx),
            resolution: CandidateResolution::Ready(default_intent),
            score: 76,
        });
    }

    let trash_targets = ctx.trash_target_paths();
    if !trash_targets.is_empty() {
        let trash_intent = FileExplorerIntent::MoveToTrash {
            targets: trash_targets,
        };
        list.push(PaletteCandidate {
            slash_syntax: "/trash",
            title: "Move Selection to Trash".to_string(),
            subtitle: "Move selected file(s) or folder(s) to system Trash".to_string(),
            preview_chip: trash_intent.preview_chip(ctx),
            resolution: CandidateResolution::Ready(trash_intent),
            score: 74,
        });
    }

    // 8. /reveal and /cd
    let reveal_path = ctx
        .primary_target_path()
        .unwrap_or_else(|| ctx.cwd.to_path_buf());
    let reveal_intent = FileExplorerIntent::Reveal { path: reveal_path };
    list.push(PaletteCandidate {
        slash_syntax: "/reveal",
        title: "Reveal in System File Manager".to_string(),
        subtitle: "Highlight entry in Finder / system file manager".to_string(),
        preview_chip: reveal_intent.preview_chip(ctx),
        resolution: CandidateResolution::Ready(reveal_intent),
        score: 72,
    });

    let cd_intent = FileExplorerIntent::CdTerminal {
        path: ctx.cd_target_path(),
    };
    list.push(PaletteCandidate {
        slash_syntax: "/cd",
        title: "cd Linked Terminal Here".to_string(),
        subtitle: "Send cd command to linked terminal pane".to_string(),
        preview_chip: cd_intent.preview_chip(ctx),
        resolution: CandidateResolution::Ready(cd_intent),
        score: 70,
    });

    list
}

fn score_catalog_candidate(candidate: &PaletteCandidate, query_norm: &str) -> Option<i32> {
    let syntax = candidate
        .slash_syntax
        .trim_start_matches('/')
        .to_ascii_lowercase();
    let title = candidate.title.to_ascii_lowercase();
    let subtitle = candidate.subtitle.to_ascii_lowercase();

    if syntax.starts_with(query_norm) {
        return Some(500 + candidate.score);
    }
    if let CandidateResolution::NeedsSlot { prompt_prefix, .. } = candidate.resolution {
        let prefix_norm = prompt_prefix
            .trim_start_matches('/')
            .trim()
            .to_ascii_lowercase();
        if prefix_norm.starts_with(query_norm) || query_norm.starts_with(&prefix_norm) {
            return Some(480 + candidate.score);
        }
    }
    if title.starts_with(query_norm) {
        return Some(400 + candidate.score);
    }
    if syntax.contains(query_norm) || title.contains(query_norm) || subtitle.contains(query_norm) {
        return Some(300 + candidate.score);
    }

    // Multi-word token match
    let tokens: Vec<&str> = query_norm.split_whitespace().collect();
    if !tokens.is_empty()
        && tokens
            .iter()
            .all(|tok| syntax.contains(tok) || title.contains(tok) || subtitle.contains(tok))
    {
        return Some(220 + candidate.score);
    }

    None
}

fn split_first_word(s: &str) -> (&str, &str) {
    let trimmed = s.trim();
    match trimmed.find(char::is_whitespace) {
        Some(idx) => (&trimmed[..idx], trimmed[idx..].trim()),
        None => (trimmed, ""),
    }
}

fn parse_optional_bool_flag(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "on" | "true" | "1" | "yes" | "show" | "enable" | "enabled" => Some(true),
        "off" | "false" | "0" | "no" | "hide" | "disable" | "disabled" => Some(false),
        _ => None,
    }
}

fn parse_file_name_and_open_flag(raw: &str) -> Option<(String, bool)> {
    let mut cursor = raw.trim();
    let mut open_in_editor = true;

    for suffix in [" --no-open", " without opening"] {
        if let Some(stripped) = strip_suffix_case_insensitive(cursor, suffix) {
            cursor = stripped.trim();
            open_in_editor = false;
        }
    }
    for suffix in [
        " --open",
        " and open in editor",
        " and open",
        " in editor",
        " in pane",
    ] {
        if let Some(stripped) = strip_suffix_case_insensitive(cursor, suffix) {
            cursor = stripped.trim();
            open_in_editor = true;
        }
    }

    let name = sanitize_single_name(cursor)?;
    Some((name, open_in_editor))
}

fn sanitize_single_name(raw: &str) -> Option<String> {
    let unquoted = strip_surrounding_quotes(raw.trim()).trim();
    if unquoted.is_empty() {
        return None;
    }
    // Reject placeholder tokens or path traversal names.
    if unquoted.starts_with('<') && unquoted.ends_with('>') {
        return None;
    }
    if matches!(unquoted, "." | "..")
        || unquoted.contains('/')
        || unquoted.contains('\\')
        || unquoted.contains('\0')
    {
        return None;
    }
    Some(unquoted.to_string())
}

fn strip_surrounding_quotes(s: &str) -> &str {
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"'))
            || (s.starts_with('\'') && s.ends_with('\''))
            || (s.starts_with('`') && s.ends_with('`')))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

fn is_selection_pronoun(s: &str) -> bool {
    matches!(
        s.trim().to_ascii_lowercase().as_str(),
        "this"
            | "it"
            | "selected"
            | "selection"
            | "current"
            | "this file"
            | "this folder"
            | "this item"
            | "selected file"
            | "selected folder"
            | "selected item"
    )
}

fn looks_like_filename(name: &str) -> bool {
    name.contains('.')
        || matches!(
            name,
            "Makefile" | "Dockerfile" | "README" | "LICENSE" | "CHANGELOG" | "justfile"
        )
}

fn strip_prefix_case_insensitive<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    if text.len() < prefix.len() {
        return None;
    }
    let (head, tail) = text.split_at(prefix.len());
    if head.eq_ignore_ascii_case(prefix) {
        Some(tail)
    } else {
        None
    }
}

fn strip_suffix_case_insensitive<'a>(text: &'a str, suffix: &str) -> Option<&'a str> {
    if text.len() < suffix.len() {
        return None;
    }
    let split_at = text.len() - suffix.len();
    let (head, tail) = text.split_at(split_at);
    if tail.eq_ignore_ascii_case(suffix) {
        Some(head)
    } else {
        None
    }
}

fn strip_word_prefix_case_insensitive<'a>(text: &'a str, word: &str) -> Option<&'a str> {
    let after = strip_prefix_case_insensitive(text, word)?;
    if after.is_empty() {
        Some(after)
    } else if after.starts_with(char::is_whitespace) {
        Some(after.trim_start())
    } else {
        None
    }
}

fn split_case_insensitive_delimiter<'a>(text: &'a str, delimiter: &str) -> Option<(&'a str, &'a str)> {
    let lower = text.to_ascii_lowercase();
    let delim_lower = delimiter.to_ascii_lowercase();
    let idx = lower.rfind(&delim_lower)?;
    let lhs = text[..idx].trim();
    let rhs = text[idx + delimiter.len()..].trim();
    if lhs.is_empty() || rhs.is_empty() {
        None
    } else {
        Some((lhs, rhs))
    }
}
