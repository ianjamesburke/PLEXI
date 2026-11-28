//! The palette's Files source: one file index per context root, walked with
//! `ignore` on a background thread and kept for the next open, so
//! cached rows answer at once while a fresh walk runs
//! (`docs/palette-picker.md`, Files). Queries are scored on the worker too;
//! the UI thread only installs answers, in `App::logic`.

use super::preview::classify;
use super::usage::UsageBook;
use crate::host::picker::{
    usage_rank_key, FileMatchRank, PreviewRef, RowKey, UsageEntry, FILES_SOURCE, MAX_SOURCE_ROWS,
};
use crate::search::{normalize_rel_path, score_candidate, CandidateInput, MatchScore, WalkConfig};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

/// One indexed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexEntry {
    /// Canonical absolute path: the row's stable id.
    pub path: PathBuf,
    /// Path relative to the walked root, `/`-separated.
    pub rel_path: String,
    pub file_name: String,
    pub depth: usize,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
}

/// Walks one context root. Production uses [`walk_root`]; tests substitute
/// a walk they can hold open.
pub(crate) type WalkFn = Arc<dyn Fn(&Path) -> Result<Vec<IndexEntry>, String> + Send + Sync>;

/// Index regular files under the canonical root, applying Git's ignore rules
/// and the shared search prune list and budgets.
pub(crate) fn walk_root(root: &Path) -> Result<Vec<IndexEntry>, String> {
    let root = root
        .canonicalize()
        .map_err(|err| format!("resolve context root {}: {err}", root.display()))?;
    Ok(walk_files(&WalkConfig::new(&root)))
}

fn walk_files(config: &WalkConfig<'_>) -> Vec<IndexEntry> {
    let root = config.root;
    let mut matcher = ignore::WalkBuilder::new(root)
        .build_matchers()
        .pop()
        .expect("one ignore matcher for the context root");
    let mut queue = VecDeque::from([(PathBuf::new(), 0)]);
    let mut entries = Vec::new();
    let mut dirs_visited = 0;
    let mut budget_hit = false;
    'walk: while let Some((dir, depth)) = queue.pop_front() {
        if depth >= config.max_depth {
            continue;
        }
        if dirs_visited >= config.max_dirs || entries.len() >= config.max_files {
            budget_hit = true;
            break;
        }
        dirs_visited += 1;

        let path = root.join(&dir);
        let Ok(children) = fs::read_dir(&path).inspect_err(|err| {
            log::warn!("palette files: read directory {}: {err}", path.display())
        }) else {
            continue;
        };
        let mut children: Vec<_> = children
            .filter_map(|child| {
                child
                    .inspect_err(|err| {
                        log::warn!("palette files: read entry in {}: {err}", path.display());
                    })
                    .ok()
            })
            .collect();
        children.sort_by_cached_key(fs::DirEntry::file_name);

        for child in children {
            let Ok(name) = child.file_name().into_string() else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            let Ok(kind) = child.file_type().inspect_err(|err| {
                log::warn!(
                    "palette files: file type for {}: {err}",
                    child.path().display()
                );
            }) else {
                continue;
            };
            let is_dir = kind.is_dir();
            if is_dir {
                if depth + 1 >= config.max_depth || config.is_pruned_dir(&name) {
                    continue;
                }
            } else if !kind.is_file() {
                continue;
            }

            let relative_path = dir.join(&name);
            let (matched, err) = matcher.matched_with_errors(&relative_path, is_dir);
            if let Some(err) = err {
                log::warn!(
                    "palette files: ignore rules for {}: {err}",
                    child.path().display()
                );
            }
            if matched.is_ignore() {
                continue;
            }
            if is_dir {
                // Queue only directories that passed ignore checks. Their
                // contents are opened once, when they reach the queue front.
                queue.push_back((relative_path, depth + 1));
                continue;
            }

            let metadata = child
                .metadata()
                .inspect_err(|err| {
                    log::warn!(
                        "palette files: metadata for {}: {err}",
                        child.path().display()
                    );
                })
                .ok();
            entries.push(IndexEntry {
                rel_path: normalize_rel_path(&relative_path),
                file_name: name,
                depth: depth + 1,
                path: child.path(),
                size: metadata.as_ref().map(fs::Metadata::len),
                modified: metadata.and_then(|meta| meta.modified().ok()),
            });
            if entries.len() == config.max_files {
                budget_hit = true;
                break 'walk;
            }
        }
    }
    if budget_hit {
        log::info!(
            "palette files: walk of {} stopped at its budget ({} files)",
            root.display(),
            entries.len()
        );
    }
    entries
}

/// How `score` ranks `entry`'s filename, or `None` when the name did not match.
fn file_match_rank(entry: &IndexEntry, score: &MatchScore) -> Option<FileMatchRank> {
    if score.basename_tier == 0 {
        return None;
    }
    let name_len = entry.file_name.len();
    let stem_len = Path::new(&entry.file_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map_or(name_len, str::len);
    let prefix_end = match score.primary_ranges.as_slice() {
        [range] if range.start == 0 => Some(range.end),
        _ => None,
    };
    let unmatched_prefix_tail = prefix_end.map_or(0, |end| {
        if end <= stem_len {
            stem_len - end
        } else {
            name_len - end
        }
    });
    Some(FileMatchRank::new(
        prefix_end == Some(stem_len) || prefix_end == Some(name_len),
        score.basename_tier,
        entry.depth,
        unmatched_prefix_tail,
    ))
}

/// A file row of an answer, ranked.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FileHit {
    pub key: RowKey,
    pub path: PathBuf,
    pub rel_path: String,
    pub file_name: String,
    pub score: Option<i64>,
    pub file_match: Option<FileMatchRank>,
    pub usage: Option<UsageEntry>,
    pub modified: Option<u64>,
    pub preview: PreviewRef,
}

/// One answer to a query. `done` is false while a walk of its root runs:
/// the hits came from the cache (or there is no cache yet).
#[derive(Debug)]
pub(crate) struct FilesAnswer {
    pub generation: u64,
    pub hits: Vec<FileHit>,
    pub done: bool,
    /// The thread that walked the index these hits came from.
    #[cfg(test)]
    pub walked_on: Option<thread::ThreadId>,
}

/// A file row's key: the source id and the canonical path.
pub(crate) fn file_key(path: &Path) -> Option<RowKey> {
    path.to_str().map(|id| RowKey::new(FILES_SOURCE, id))
}

struct Query {
    root: PathBuf,
    text: String,
    generation: u64,
    usage: UsageBook,
}

enum Message {
    /// The owner dropped the index. The worker holds a sender of its own
    /// (for walk results), so a closed channel alone never stops it.
    Stop,
    Refresh(PathBuf),
    Query(Query),
    Walked {
        root: PathBuf,
        result: Result<Vec<IndexEntry>, String>,
        #[cfg(test)]
        thread: thread::ThreadId,
    },
}

struct Index {
    entries: Vec<IndexEntry>,
    #[cfg(test)]
    walked_on: thread::ThreadId,
}

/// The index worker. Owned by the host; dropping it stops the worker. A
/// walk still running finishes on its own thread and is discarded.
pub(crate) struct FileIndex {
    messages: Sender<Message>,
    answers: Receiver<FilesAnswer>,
}

impl FileIndex {
    pub(crate) fn spawn(
        walk: WalkFn,
        wake: Arc<dyn crate::app::ui_mailbox::UiWake>,
    ) -> Result<Self, String> {
        let (messages, inbox) = mpsc::channel();
        let (answer_tx, answers) = mpsc::channel();
        let walk_tx = messages.clone();
        thread::Builder::new()
            .name("plexi-palette-files".to_owned())
            .spawn(move || {
                Worker {
                    walk,
                    wake,
                    walk_tx,
                    answers: answer_tx,
                    cache: HashMap::new(),
                    walking: HashSet::new(),
                    latest: None,
                }
                .run(inbox)
            })
            .map_err(|err| format!("spawn palette files worker: {err}"))?;
        Ok(Self { messages, answers })
    }

    /// Walk `root` again in the background (the palette opened).
    pub(crate) fn refresh(&self, root: PathBuf) {
        self.send(Message::Refresh(root));
    }

    /// Answer `text` over `root`'s index, ranked with `usage`.
    pub(crate) fn query(&self, root: PathBuf, text: String, generation: u64, usage: UsageBook) {
        self.send(Message::Query(Query {
            root,
            text,
            generation,
            usage,
        }));
    }

    /// Answers that arrived since the last call. Serviced from `App::logic`.
    pub(crate) fn answers(&self) -> Vec<FilesAnswer> {
        self.answers.try_iter().collect()
    }

    fn send(&self, message: Message) {
        if self.messages.send(message).is_err() {
            log::warn!("palette files: worker is gone; the Files stop cannot answer");
        }
    }
}

impl Drop for FileIndex {
    fn drop(&mut self) {
        // Gone already is fine: the worker only exits on Stop or a panic.
        let _stopped = self.messages.send(Message::Stop);
    }
}

struct Worker {
    walk: WalkFn,
    wake: Arc<dyn crate::app::ui_mailbox::UiWake>,
    walk_tx: Sender<Message>,
    answers: Sender<FilesAnswer>,
    cache: HashMap<PathBuf, Index>,
    walking: HashSet<PathBuf>,
    latest: Option<Query>,
}

impl Worker {
    fn run(mut self, inbox: Receiver<Message>) {
        while let Ok(message) = inbox.recv() {
            match message {
                Message::Stop => return,
                Message::Refresh(root) => self.start_walk(root),
                Message::Query(query) => {
                    if !self.cache.contains_key(&query.root) {
                        self.start_walk(query.root.clone());
                    }
                    self.answer(&query);
                    self.latest = Some(query);
                }
                Message::Walked {
                    root,
                    result,
                    #[cfg(test)]
                    thread,
                } => {
                    self.walking.remove(&root);
                    match result {
                        Ok(entries) => {
                            log::info!("palette files: indexed {} file(s)", entries.len());
                            self.cache.insert(
                                root.clone(),
                                Index {
                                    entries,
                                    #[cfg(test)]
                                    walked_on: thread,
                                },
                            );
                        }
                        Err(err) => log::warn!("palette files: walk failed: {err}"),
                    }
                    if let Some(query) = self.latest.take() {
                        if query.root == root {
                            self.answer(&query);
                        }
                        self.latest = Some(query);
                    }
                }
            }
        }
    }

    fn start_walk(&mut self, root: PathBuf) {
        if !self.walking.insert(root.clone()) {
            return;
        }
        let walk = Arc::clone(&self.walk);
        let done = self.walk_tx.clone();
        let spawned = thread::Builder::new()
            .name("plexi-palette-walk".to_owned())
            .spawn(move || {
                let result = walk(&root);
                // The worker is gone only when the host shut the index down.
                let _closed = done.send(Message::Walked {
                    root,
                    result,
                    #[cfg(test)]
                    thread: thread::current().id(),
                });
            });
        if let Err(err) = spawned {
            log::warn!("palette files: spawn walk: {err}");
        }
    }

    fn answer(&self, query: &Query) {
        let walking = self.walking.contains(&query.root);
        let index = self.cache.get(&query.root);
        let hits = index.map_or_else(Vec::new, |index| rank(&index.entries, query));
        let answer = FilesAnswer {
            generation: query.generation,
            hits,
            done: !walking,
            #[cfg(test)]
            walked_on: index.map(|index| index.walked_on),
        };
        if self.answers.send(answer).is_ok() {
            self.wake.wake("palette_files");
        }
    }
}

/// Score every entry against the query and keep the best `MAX_SOURCE_ROWS`
/// by the same key the reducer's `RankingPreset::Usage` sorts with.
fn rank(entries: &[IndexEntry], query: &Query) -> Vec<FileHit> {
    let now = SystemTime::now();
    let text = query.text.trim();
    let mut hits: Vec<FileHit> = entries
        .iter()
        .filter_map(|entry| {
            let key = file_key(&entry.path)?;
            let score = if text.is_empty() {
                None
            } else {
                let candidate = CandidateInput::new(&entry.file_name)
                    .with_relative_path(&entry.rel_path)
                    .with_mtime(entry.modified)
                    .with_depth(entry.depth);
                Some(score_candidate(text, &candidate, now)?)
            };
            let usage = query.usage.get(key.as_str());
            Some(FileHit {
                usage,
                file_match: score
                    .as_ref()
                    .and_then(|score| file_match_rank(entry, score)),
                score: score.map(|score| score.total),
                modified: entry
                    .modified
                    .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
                    .map(|age| age.as_secs()),
                preview: classify(&entry.path, entry.size),
                path: entry.path.clone(),
                rel_path: entry.rel_path.clone(),
                file_name: entry.file_name.clone(),
                key,
            })
        })
        .collect();
    hits.sort_by_cached_key(|hit| {
        usage_rank_key(hit.file_match, hit.score, hit.usage, hit.modified)
    });
    hits.truncate(MAX_SOURCE_ROWS);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    struct NoWake;
    impl crate::app::ui_mailbox::UiWake for NoWake {
        fn wake(&self, _source: &'static str) {}
    }

    /// The index and the answers drained from it but not yet read.
    struct Answers<'a>(&'a FileIndex, std::collections::VecDeque<FilesAnswer>);

    impl Answers<'_> {
        fn next(&mut self) -> FilesAnswer {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(answer) = self.1.pop_front() {
                    return answer;
                }
                self.1.extend(self.0.answers());
                assert!(
                    Instant::now() < deadline || !self.1.is_empty(),
                    "no files answer arrived"
                );
                thread::sleep(Duration::from_millis(5));
            }
        }
    }

    fn names(answer: &FilesAnswer) -> Vec<&str> {
        answer
            .hits
            .iter()
            .map(|hit| hit.rel_path.as_str())
            .collect()
    }

    fn readme_workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("workspace");
        let root = dir.path();
        for path in ["a/b", "ignored", "worktrees/feature", ".git/info"] {
            std::fs::create_dir_all(root.join(path)).expect("fixture directory");
        }
        for path in [
            "README.md",
            "a/b/README.md",
            "readfile.tcl",
            "readme-guide.md",
            "ignored/README.md",
            "worktrees/feature/README.md",
        ] {
            std::fs::write(root.join(path), "fixture").expect("fixture file");
        }
        std::fs::write(root.join(".gitignore"), "/ignored/\n").expect("gitignore");
        std::fs::write(root.join(".git/info/exclude"), "/worktrees/\n").expect("git exclude");
        std::fs::File::open(root.join("README.md"))
            .expect("open root README")
            .set_modified(UNIX_EPOCH + Duration::from_secs(1))
            .expect("make root README older than the other files");
        dir
    }

    #[test]
    fn files_rank_shallow_short_names_first() {
        let dir = readme_workspace();
        let entries = walk_root(dir.path()).expect("walk");
        for text in ["read", "readme"] {
            let hits = rank(
                &entries,
                &Query {
                    root: dir.path().to_path_buf(),
                    text: text.to_owned(),
                    generation: 1,
                    usage: UsageBook::default(),
                },
            );
            let names: Vec<_> = hits.iter().map(|hit| hit.rel_path.as_str()).collect();
            assert_eq!(names.first(), Some(&"README.md"), "{text}: {names:?}");
            if text == "readme" {
                assert_eq!(names, ["README.md", "a/b/README.md", "readme-guide.md"]);
            }
        }
    }

    #[test]
    fn files_honour_gitignore_and_git_exclude() {
        let dir = readme_workspace();
        let entries = walk_root(dir.path()).expect("walk");
        let mut names: Vec<_> = entries
            .iter()
            .map(|entry| entry.rel_path.as_str())
            .collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "README.md",
                "a/b/README.md",
                "readfile.tcl",
                "readme-guide.md"
            ]
        );
    }

    #[test]
    fn files_index_shallow_siblings_before_deep_files_under_budget() {
        let dir = tempfile::tempdir().expect("workspace");
        let root = dir.path().canonicalize().expect("canonical root");
        std::fs::create_dir_all(root.join("a/deep")).expect("deep directory");
        std::fs::create_dir(root.join("z")).expect("later directory");
        for n in 0..8 {
            std::fs::write(root.join(format!("a/deep/{n}.txt")), "deep").expect("deep file");
        }
        std::fs::write(root.join("z/README.md"), "shallow").expect("shallow file");

        let entries = walk_files(&WalkConfig::new(&root).with_max_files(2));
        let names: Vec<_> = entries
            .iter()
            .map(|entry| entry.rel_path.as_str())
            .collect();
        assert_eq!(names, ["z/README.md", "a/deep/0.txt"]);
    }

    #[test]
    fn file_walk_never_runs_on_the_ui_thread() {
        let dir = tempfile::tempdir().expect("dir");
        std::fs::write(dir.path().join("a.txt"), "a").expect("write");
        let index = FileIndex::spawn(Arc::new(walk_root), Arc::new(NoWake)).expect("spawn");
        let mut answers = Answers(&index, Default::default());
        index.query(
            dir.path().to_path_buf(),
            String::new(),
            1,
            UsageBook::default(),
        );
        let first = answers.next();
        assert!(!first.done && first.hits.is_empty(), "no cache: walking");
        let walked = answers.next();
        assert!(walked.done);
        assert_eq!(names(&walked), ["a.txt"]);
        let walked_on = walked.walked_on.expect("walk thread recorded");
        assert_ne!(walked_on, thread::current().id());
    }

    #[cfg(unix)]
    #[test]
    fn files_stay_inside_the_active_context_root() {
        let outside = tempfile::tempdir().expect("outside");
        std::fs::write(outside.path().join("secret.txt"), "s").expect("write");
        let dir = tempfile::tempdir().expect("dir");
        let root = dir.path().join("ctx");
        std::fs::create_dir_all(root.join("src")).expect("mkdir");
        std::fs::write(root.join("src/main.rs"), "fn main() {}").expect("write");
        std::fs::write(dir.path().join("sibling.txt"), "x").expect("write");
        std::os::unix::fs::symlink(outside.path(), root.join("linked")).expect("dir link");
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            root.join("secret-link.txt"),
        )
        .expect("file link");

        let entries = walk_root(&root).expect("walk");
        let canonical = root.canonicalize().expect("canonical root");
        assert_eq!(
            entries
                .iter()
                .map(|e| e.rel_path.as_str())
                .collect::<Vec<_>>(),
            ["src/main.rs"]
        );
        for entry in &entries {
            assert!(entry.path.starts_with(&canonical), "{entry:?}");
            assert_eq!(
                file_key(&entry.path).expect("utf-8").as_str(),
                format!("files:{}", canonical.join("src/main.rs").display())
            );
        }
    }

    #[test]
    fn cached_rows_show_before_the_walk_finishes() {
        let walks = Arc::new(std::sync::Mutex::new(0usize));
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = Arc::new(std::sync::Mutex::new(release_rx));
        let walk: WalkFn = {
            let walks = Arc::clone(&walks);
            Arc::new(move |root: &Path| {
                let n = {
                    let mut walks = walks.lock().expect("walks");
                    *walks += 1;
                    *walks
                };
                if n > 1 {
                    // The second walk is slow: held until the test releases it.
                    let _released = release_rx.lock().expect("gate").recv();
                }
                Ok(vec![IndexEntry {
                    path: root.join(format!("walk{n}.txt")),
                    rel_path: format!("walk{n}.txt"),
                    file_name: format!("walk{n}.txt"),
                    depth: 1,
                    size: Some(1),
                    modified: None,
                }])
            })
        };
        let root = PathBuf::from("/ctx");
        let index = FileIndex::spawn(walk, Arc::new(NoWake)).expect("spawn");
        let mut answers = Answers(&index, Default::default());
        index.query(root.clone(), String::new(), 1, UsageBook::default());
        let _walking = answers.next();
        assert_eq!(names(&answers.next()), ["walk1.txt"]);

        // The palette opens again: a refresh walk starts and is held.
        index.refresh(root.clone());
        index.query(root.clone(), String::new(), 2, UsageBook::default());
        let cached = answers.next();
        assert_eq!(cached.generation, 2);
        assert!(!cached.done, "the walk is still running");
        assert_eq!(names(&cached), ["walk1.txt"], "cached rows answer at once");

        release_tx.send(()).expect("release the walk");
        let fresh = answers.next();
        assert!(fresh.done);
        assert_eq!(names(&fresh), ["walk2.txt"]);
    }

    fn entry(name: &str, modified: u64) -> IndexEntry {
        IndexEntry {
            path: PathBuf::from("/ctx").join(name),
            rel_path: name.to_owned(),
            file_name: name.to_owned(),
            depth: 1,
            size: Some(1),
            modified: Some(UNIX_EPOCH + Duration::from_secs(modified)),
        }
    }

    #[test]
    fn source_cuts_its_answer_by_the_reducers_order() {
        let entries: Vec<IndexEntry> = (0..MAX_SOURCE_ROWS as u64 + 10)
            .map(|i| entry(&format!("f{i}.txt"), i))
            .collect();
        let mut usage = UsageBook::default();
        usage.record("files:/ctx/f0.txt", SystemTime::now());
        let query = Query {
            root: PathBuf::from("/ctx"),
            text: String::new(),
            generation: 1,
            usage,
        };
        let hits = rank(&entries, &query);
        assert_eq!(hits.len(), MAX_SOURCE_ROWS);
        assert_eq!(
            hits[0].rel_path, "f0.txt",
            "the used file leads, though oldest"
        );
        let newest = format!("f{}.txt", MAX_SOURCE_ROWS + 9);
        assert_eq!(hits[1].rel_path, newest, "then newest modified");
    }
}
