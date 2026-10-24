use super::parser::{parse_report, Osc7501Scanner, ScanEvent, FEATURE_REPLY, MAX_SEQUENCE};
use super::records::{Kind, Report, State};
use super::{ProgramStatusStore, ProgramStatusTap};
use base64::Engine;
use egui_term::{ByteTapHandle, PtyByteTap, TapReader};
use std::io::Read;
use std::sync::{Arc, Mutex};

fn b64(s: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(s)
}

fn osc(payload: &str) -> Vec<u8> {
    format!("\x1b]7501;{payload}\x07").into_bytes()
}

fn tap() -> (ProgramStatusTap, Arc<Mutex<ProgramStatusStore>>) {
    let store = Arc::new(Mutex::new(ProgramStatusStore::default()));
    (ProgramStatusTap::new(1, store.clone()), store)
}

fn feed(t: &mut ProgramStatusTap, bytes: &[u8]) -> Vec<u8> {
    t.on_bytes(bytes)
}

fn rec(
    store: &Arc<Mutex<ProgramStatusStore>>,
    id: &str,
) -> Option<super::records::Record> {
    store.lock().unwrap().get(id).cloned()
}

fn set(p: &str) -> (String, super::records::Record) {
    match parse_report(p.as_bytes()) {
        Some(Report::Set { id, record }) => (id, record),
        other => panic!("expected Set for {p:?}, got {other:?}"),
    }
}

#[test]
fn ps7501_basic_report_bel_and_st() {
    let (mut t, s) = tap();
    feed(&mut t, &osc("state=working"));
    assert_eq!(rec(&s, "").unwrap().state, State::Working);
    feed(&mut t, b"\x1b]7501;state=done\x1b\\");
    assert_eq!(rec(&s, "").unwrap().state, State::Done);
}

#[test]
fn ps7501_split_across_reads() {
    let (mut t, s) = tap();
    let full = osc("state=blocked:kind=permission");
    for chunk in full.chunks(1) {
        feed(&mut t, chunk);
    }
    assert_eq!(rec(&s, "").unwrap().kind, Some(Kind::Permission));
    let full = b"\x1b]7501;state=idle\x1b\\".to_vec();
    let (a, b) = full.split_at(full.len() - 1);
    feed(&mut t, a);
    assert_eq!(rec(&s, "").unwrap().state, State::Blocked);
    feed(&mut t, b);
    assert_eq!(rec(&s, "").unwrap().state, State::Idle);
}

#[test]
fn ps7501_malformed_pairs_skipped() {
    let (_, r) = set("novalue:=x:state=working:bad!=1:app=a b:progress=5");
    assert_eq!(r.state, State::Working);
    assert_eq!(r.progress, Some(5));
    assert_eq!(r.app, None);
    let (_, r) = set("state=working:msg=@@@:UP=1");
    assert_eq!(r.msg, None);
}

#[test]
fn ps7501_unknown_keys_ignored_last_repeat_wins() {
    let (_, r) = set("state=idle:future=1:state=working:app=a:app=b");
    assert_eq!(r.state, State::Working);
    assert_eq!(r.app.as_deref(), Some("b"));
}

#[test]
fn ps7501_whitespace_trimmed() {
    let (_, r) = set(" state = working : app = cargo ");
    assert_eq!(r.state, State::Working);
    assert_eq!(r.app.as_deref(), Some("cargo"));
}

#[test]
fn ps7501_state_required_and_unknown_dropped() {
    assert!(parse_report(b"app=x").is_none());
    assert!(parse_report(b"state=sleeping").is_none());
    assert!(parse_report(b"state=").is_none());
}

#[test]
fn ps7501_msg_title_base64_roundtrip_and_optional_padding() {
    let p = format!(
        "state=done:msg={}:title={}",
        b64("Apply 3 to add, 1 to change?"),
        "VVMgRWFzdA"
    );
    let (_, r) = set(&p);
    assert_eq!(r.msg.as_deref(), Some("Apply 3 to add, 1 to change?"));
    assert_eq!(r.title.as_deref(), Some("US East"));
    let (_, r) = set(&format!("state=done:msg={}", b64("héllo ✓")));
    assert_eq!(r.msg.as_deref(), Some("héllo ✓"));
}

#[test]
fn ps7501_bad_base64_discards_whole_report() {
    assert!(parse_report(b"state=working:msg=a").is_none());
    assert!(parse_report(b"state=working:msg=ab-d").is_none());
    assert!(parse_report(b"state=working:title=!!").is_some()); // `!` malformed, skipped
}

#[test]
fn ps7501_control_chars_discard_whole_report() {
    for bad in ["a\nb", "a\tb", "a\x7fb", "a\u{85}b", "a\0b"] {
        let p = format!("state=working:msg={}", b64(bad));
        assert!(parse_report(p.as_bytes()).is_none(), "{bad:?}");
        let p = format!("state=working:title={}", b64(bad));
        assert!(parse_report(p.as_bytes()).is_none(), "{bad:?}");
    }
    // Invalid UTF-8 is also discarded.
    let raw = base64::engine::general_purpose::STANDARD.encode([0xffu8, 0xfe]);
    assert!(parse_report(format!("state=idle:msg={raw}").as_bytes()).is_none());
}

#[test]
fn ps7501_limits() {
    let ok = "a".repeat(2048);
    assert!(parse_report(format!("state=idle:msg={}", b64(&ok)).as_bytes()).is_some());
    let big = "a".repeat(2049);
    assert!(parse_report(format!("state=idle:msg={}", b64(&big)).as_bytes()).is_none());
    // Encoded-size check happens before decoding.
    let enc = "A".repeat(2733);
    assert!(parse_report(format!("state=idle:msg={enc}").as_bytes()).is_none());
    let ok = "a".repeat(192);
    assert!(parse_report(format!("state=idle:title={}", b64(&ok)).as_bytes()).is_some());
    let big = "a".repeat(193);
    assert!(parse_report(format!("state=idle:title={}", b64(&big)).as_bytes()).is_none());
    let enc = "A".repeat(257);
    assert!(parse_report(format!("state=idle:title={enc}").as_bytes()).is_none());
    // Key length.
    assert!(parse_report(b"state=idle:abcdefghijklmnop=1").is_some());
    assert!(parse_report(b"state=idle:abcdefghijklmnopq=1").is_none());
}

#[test]
fn ps7501_oversize_sequence_dropped_by_scanner() {
    let mut sc = Osc7501Scanner::new();
    let pad = "a".repeat(MAX_SEQUENCE);
    let ev = sc.feed(&osc(&format!("state=idle:x={pad}")));
    assert!(ev.is_empty());
    // Exactly at the cap is accepted: ESC ] 7501 ; + payload + BEL = 4096.
    let overhead = 2 + 5 + 1;
    let payload = format!("state=idle:x={}", "a".repeat(MAX_SEQUENCE - overhead - 13));
    assert_eq!(payload.len() + overhead, MAX_SEQUENCE);
    assert_eq!(sc.feed(&osc(&payload)).len(), 1);
    let payload = format!("{payload}a");
    assert!(sc.feed(&osc(&payload)).is_empty());
    // Scanner recovers.
    assert_eq!(sc.feed(&osc("state=idle")).len(), 1);
}

#[test]
fn ps7501_kind_only_with_blocked() {
    assert_eq!(set("state=blocked:kind=auth").1.kind, Some(Kind::Auth));
    assert_eq!(set("state=blocked:kind=question").1.kind, Some(Kind::Question));
    assert_eq!(set("state=working:kind=permission").1.kind, None);
    assert_eq!(set("state=blocked:kind=bogus").1.kind, None);
    assert_eq!(set("state=blocked:kind=bogus").1.state, State::Blocked);
}

#[test]
fn ps7501_progress_range() {
    assert_eq!(set("state=working:progress=0").1.progress, Some(0));
    assert_eq!(set("state=working:progress=100").1.progress, Some(100));
    assert_eq!(set("state=working:progress=101").1.progress, None);
    assert_eq!(set("state=working:progress=-1").1.progress, None);
    assert_eq!(set("state=working:progress=5.5").1.progress, None);
    assert_eq!(set("state=working:progress=+5").1.progress, None);
    assert_eq!(set("state=blocked:progress=50").1.progress, Some(50));
    assert_eq!(set("state=done:progress=50").1.progress, None);
    assert_eq!(set("state=idle:progress=50").1.progress, None);
    assert_eq!(set("state=working:progress=50").1.state, State::Working);
}

#[test]
fn ps7501_invalid_app_treated_as_absent() {
    assert_eq!(set("state=idle:app=a.b_c+d-e").1.app.as_deref(), Some("a.b_c+d-e"));
    assert_eq!(set("state=idle:app=a/b").1.app, None);
    let long = "a".repeat(33);
    assert_eq!(set(&format!("state=idle:app={long}")).1.app, None);
    assert_eq!(set("state=idle:app=").1.app, None);
}

#[test]
fn ps7501_id_grammar() {
    assert_eq!(set("state=idle").0, "");
    assert_eq!(set("state=idle:id=build/test").0, "build/test");
    let seg = "a".repeat(32);
    assert_eq!(set(&format!("state=idle:id={seg}")).0, seg);
    for bad in [
        "".to_string(),
        "a//b".into(),
        "/a".into(),
        "a/".into(),
        "a b".into(),
        "a".repeat(33),
        "a/b/c/d/e/f/g/h/i".into(),
        format!("{0}/{0}/{0}/{0}/x", "a".repeat(32)),
    ] {
        assert!(
            parse_report(format!("state=idle:id={bad}").as_bytes()).is_none(),
            "{bad:?}"
        );
    }
    assert!(parse_report(b"state=idle:id=a/b/c/d/e/f/g/h").is_some());
    assert!(parse_report(b"state=clear:id=a//b").is_none());
}

#[test]
fn ps7501_bad_id_never_falls_back_to_root() {
    let (mut t, s) = tap();
    feed(&mut t, &osc("state=working:msg=aGk="));
    feed(&mut t, &osc("state=done:id=bad//id"));
    assert_eq!(rec(&s, "").unwrap().state, State::Working);
    assert_eq!(s.lock().unwrap().len(), 1);
    feed(&mut t, &osc("state=clear:id=bad//id"));
    assert_eq!(s.lock().unwrap().len(), 1);
}

#[test]
fn ps7501_report_replaces_record_whole() {
    let (mut t, s) = tap();
    feed(&mut t, &osc(&format!("state=working:app=x:progress=3:msg={}", b64("hi"))));
    feed(&mut t, &osc("state=working"));
    let r = rec(&s, "").unwrap();
    assert_eq!((r.app, r.progress, r.msg), (None, None, None));
}

#[test]
fn ps7501_clear_subtree_and_clear_all() {
    let (mut t, s) = tap();
    for id in ["a", "a/b", "a/b/c", "ab", "z"] {
        feed(&mut t, &osc(&format!("state=working:id={id}")));
    }
    feed(&mut t, &osc("state=working"));
    feed(&mut t, &osc("state=clear:id=a"));
    let ids: Vec<String> = s.lock().unwrap().iter().map(|(k, _)| k.to_string()).collect();
    assert_eq!(ids, ["", "ab", "z"]); // prefix match is per segment, not per char
    feed(&mut t, &osc("state=clear"));
    assert!(s.lock().unwrap().is_empty());
}

#[test]
fn ps7501_app_inheritance() {
    let (mut t, s) = tap();
    feed(&mut t, &osc("state=working:app=deploy"));
    feed(&mut t, &osc("state=working:id=us/east"));
    feed(&mut t, &osc("state=working:id=us:app=mid"));
    feed(&mut t, &osc("state=working:id=eu:app="));
    let g = s.lock().unwrap();
    assert_eq!(g.effective_app("us/east"), Some("mid"));
    assert_eq!(g.effective_app("eu"), Some("deploy"));
    assert_eq!(g.effective_app(""), Some("deploy"));
    assert_eq!(g.effective_app("nope/deep"), Some("deploy"));
}

#[test]
fn ps7501_record_cap_evicts_least_recently_updated() {
    let (mut t, s) = tap();
    for i in 0..256 {
        feed(&mut t, &osc(&format!("state=idle:id=r{i}")));
    }
    feed(&mut t, &osc("state=working:id=r0")); // refresh r0
    feed(&mut t, &osc("state=idle:id=new"));
    let g = s.lock().unwrap();
    assert_eq!(g.len(), 256);
    assert!(g.get("r0").is_some());
    assert!(g.get("r1").is_none());
    assert!(g.get("new").is_some());
}

#[test]
fn ps7501_discarded_report_leaves_store_untouched() {
    let (mut t, s) = tap();
    feed(&mut t, &osc("state=working"));
    feed(&mut t, &osc("state=done:msg=a"));
    assert_eq!(rec(&s, "").unwrap().state, State::Working);
}

#[test]
fn ps7501_lifetime_exit_and_prompt() {
    for via_prompt in [false, true] {
        let (mut t, s) = tap();
        feed(&mut t, &osc("state=working:id=w"));
        feed(&mut t, &osc("state=blocked:id=b"));
        feed(&mut t, &osc("state=done:id=d"));
        feed(&mut t, &osc("state=error:id=e"));
        feed(&mut t, &osc("state=idle:id=i"));
        if via_prompt {
            feed(&mut t, b"\x1b]133;A\x07");
        } else {
            s.lock().unwrap().on_process_exit();
        }
        let g = s.lock().unwrap();
        assert!(g.get("w").is_none() && g.get("b").is_none());
        assert!(g.get("d").is_some() && g.get("e").is_some() && g.get("i").is_some());
    }
}

#[test]
fn ps7501_osc133_a_variants() {
    let mut sc = Osc7501Scanner::new();
    assert_eq!(sc.feed(b"\x1b]133;A\x1b\\"), [ScanEvent::PromptStart]);
    assert_eq!(sc.feed(b"\x1b]133;A;cl=m;aid=1\x07"), [ScanEvent::PromptStart]);
    assert!(sc.feed(b"\x1b]133;B\x07").is_empty());
    assert!(sc.feed(b"\x1b]133;C\x07").is_empty());
    assert!(sc.feed(b"\x1b]133;D;0\x07").is_empty());
    assert!(sc.feed(b"\x1b]133;AB\x07").is_empty());
}

#[test]
fn ps7501_mark_seen_drops_done_and_error() {
    let (mut t, s) = tap();
    feed(&mut t, &osc("state=done:id=d"));
    feed(&mut t, &osc("state=error:id=e"));
    feed(&mut t, &osc("state=working:id=w"));
    feed(&mut t, &osc("state=idle:id=i"));
    s.lock().unwrap().mark_seen();
    let g = s.lock().unwrap();
    assert!(g.get("d").is_none() && g.get("e").is_none());
    assert!(g.get("w").is_some() && g.get("i").is_some());
}

#[test]
fn ps7501_ris_clears_all_decstr_does_not() {
    let (mut t, s) = tap();
    feed(&mut t, &osc("state=working"));
    feed(&mut t, &osc("state=done:id=a"));
    feed(&mut t, b"\x1b[!p"); // DECSTR
    assert_eq!(s.lock().unwrap().len(), 2);
    feed(&mut t, b"\x1b[?1049h\x1b[?1049l\x1b[?47h\x1b[?1047l");
    assert_eq!(s.lock().unwrap().len(), 2);
    feed(&mut t, b"\x1b[0c\x1b(B"); // other escapes, `c` after CSI is not RIS
    assert_eq!(s.lock().unwrap().len(), 2);
    feed(&mut t, b"\x1bc");
    assert!(s.lock().unwrap().is_empty());
}

#[test]
fn ps7501_ris_split_across_reads() {
    let (mut t, s) = tap();
    feed(&mut t, &osc("state=working"));
    feed(&mut t, b"abc\x1b");
    assert_eq!(s.lock().unwrap().len(), 1);
    feed(&mut t, b"c");
    assert!(s.lock().unwrap().is_empty());
}

#[test]
fn ps7501_can_sub_abort_and_recovery() {
    for abort in [0x18u8, 0x1a] {
        let (mut t, s) = tap();
        let mut bytes = b"\x1b]7501;state=working".to_vec();
        bytes.push(abort);
        bytes.extend_from_slice(b":app=x\x07");
        feed(&mut t, &bytes);
        assert!(s.lock().unwrap().is_empty());
        feed(&mut t, &osc("state=idle"));
        assert_eq!(s.lock().unwrap().len(), 1);
    }
    // ESC not followed by `\` aborts the string; a new OSC may start right away.
    let (mut t, s) = tap();
    feed(&mut t, b"\x1b]7501;state=working\x1b\x1b]7501;state=idle\x07");
    assert_eq!(rec(&s, "").unwrap().state, State::Idle);
}

#[test]
fn ps7501_other_osc_ignored_by_scanner() {
    let mut sc = Osc7501Scanner::new();
    for seq in [
        &b"\x1b]0;title\x07"[..],
        b"\x1b]2;title\x1b\\",
        b"\x1b]7;file:///tmp\x07",
        b"\x1b]8;;http://x\x1b\\link\x1b]8;;\x1b\\",
        b"\x1b]52;c;aGk=\x07",
        b"\x1b]133;B\x07",
        b"\x1b]75012;state=working\x07",
        b"\x1b]750;state=working\x07",
        b"\x1b]07501;state=working\x07",
        b"\x1b]7501\x07",
        b"plain text \x1b[31mred\x1b[0m",
    ] {
        assert!(sc.feed(seq).is_empty(), "{seq:?}");
    }
}

#[test]
fn ps7501_feature_query_exact_reply() {
    let (mut t, s) = tap();
    let reply = feed(&mut t, b"\x1b]7501;?\x1b\\");
    assert_eq!(
        reply,
        b"\x1b]7501;?:states=idle,working,done,blocked,error:kinds=permission,question,auth\x1b\\"
    );
    assert_eq!(reply, FEATURE_REPLY);
    assert!(s.lock().unwrap().is_empty());
    assert_eq!(feed(&mut t, b"\x1b]7501;?\x07"), FEATURE_REPLY);
    assert!(feed(&mut t, &osc("state=idle")).is_empty());
    assert!(feed(&mut t, &osc("?:x=1")).is_empty());
}

#[test]
fn ps7501_tap_reader_forwards_bytes_unchanged() {
    let data: Vec<u8> = b"\x1b]0;t\x07\x1b]2;t\x1b\\\x1b]7;file:///x\x07\x1b]8;;u\x1b\\\x1b]52;c;aGk=\x07\x1b]133;A\x07\x1b]7501;?\x07hello"
        .to_vec();
    let store = Arc::new(Mutex::new(ProgramStatusStore::default()));
    let replies: Arc<Mutex<Vec<u8>>> = Arc::default();
    let sink = replies.clone();
    let mut reader = TapReader::new(
        std::io::Cursor::new(data.clone()),
        Some(ByteTapHandle::new(ProgramStatusTap::new(1, store.clone()))),
        Box::new(move |b| sink.lock().unwrap().extend_from_slice(&b)),
    );
    let mut got = Vec::new();
    let mut buf = [0u8; 7];
    loop {
        let n = reader.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        got.extend_from_slice(&buf[..n]);
    }
    assert_eq!(got, data);
    assert!(store.lock().unwrap().is_empty());
    assert_eq!(*replies.lock().unwrap(), FEATURE_REPLY);
}

#[test]
fn ps7501_tap_reader_without_tap_is_passthrough() {
    let mut reader = TapReader::new(std::io::Cursor::new(b"abc".to_vec()), None, Box::new(|_| {}));
    let mut s = String::new();
    reader.read_to_string(&mut s).unwrap();
    assert_eq!(s, "abc");
}

fn store_of(reports: &[&str]) -> ProgramStatusStore {
    let mut store = ProgramStatusStore::default();
    for r in reports {
        store.apply(parse_report(r.as_bytes()).expect("report"));
    }
    store
}

fn mapped(reports: &[&str]) -> Option<crate::protocol::PaneAgentState> {
    super::root_or_best_agent_state(&store_of(reports), 9)
}

#[test]
fn ps7501_mapping_states_and_empty() {
    use crate::protocol::AgentState as A;
    assert!(mapped(&[]).is_none());
    for (payload, want) in [
        ("state=working", A::Working),
        ("state=blocked:kind=permission", A::NeedsYou),
        ("state=blocked:kind=question", A::NeedsYou),
        ("state=blocked:kind=auth", A::NeedsYou),
        ("state=blocked", A::NeedsYou),
        ("state=idle", A::Idle),
        ("state=done", A::Completed),
        ("state=error", A::Failed),
    ] {
        let got = mapped(&[payload]).unwrap();
        assert_eq!(got.state, want, "{payload}");
        assert_eq!(got.pane_id, 9);
        assert_eq!(got.agent, "program");
    }
}

#[test]
fn ps7501_mapping_detail_text() {
    // msg = "run rm"
    let m = "msg=cnVuIHJt";
    let d = |p: &str| mapped(&[p]).unwrap().detail;
    assert_eq!(d(&format!("state=blocked:kind=permission:{m}")).as_deref(), Some("permission: run rm"));
    assert_eq!(d(&format!("state=blocked:{m}")).as_deref(), Some("run rm"));
    assert_eq!(d("state=blocked:kind=auth").as_deref(), Some("auth"));
    assert_eq!(d("state=blocked"), None);
    assert_eq!(d(&format!("state=working:{m}")).as_deref(), Some("run rm"));
}

#[test]
fn ps7501_mapping_priority_and_app_inheritance() {
    use crate::protocol::AgentState as A;
    // A blocked child beats a working root and inherits the root's app.
    let got = mapped(&["state=working:app=claude", "state=blocked:kind=question:id=sub"]).unwrap();
    assert_eq!(got.state, A::NeedsYou);
    assert_eq!(got.agent, "claude");
    // blocked > error > working > done > idle
    let order = ["idle", "done", "working", "error", "blocked"];
    for i in 1..order.len() {
        let got = mapped(&[
            &format!("state={}", order[i]),
            &format!("state={}:id=a", order[i - 1]),
        ])
        .unwrap();
        let want = mapped(&[&format!("state={}", order[i])]).unwrap();
        assert_eq!(got.state, want.state, "{} over {}", order[i], order[i - 1]);
    }
    // Tie goes to the most recently updated record.
    let got = mapped(&["state=working:app=old", "state=working:id=b:app=new"]).unwrap();
    assert_eq!(got.agent, "new");
    // An app on the record itself beats the ancestor's.
    let got = mapped(&["state=idle:app=root", "state=working:id=a:app=own"]).unwrap();
    assert_eq!(got.agent, "own");
}

#[test]
fn ps7501_encode_report_round_trips_tree_ids() {
    use super::emit::{encode_report, tree_id, APP};
    for names in [
        vec!["lead"],
        vec!["lead", "helper-1"],
        vec!["lead", "helper-1", "scout-2"],
    ] {
        let id = tree_id(&names).unwrap();
        assert_eq!(id, names.join("/"));
        let bytes = encode_report(
            Some(&id),
            State::Blocked,
            Some("question"),
            APP,
            Some("Helper One ünï"),
            Some("which branch?\nplease"),
        );
        assert!(bytes.starts_with(b"\x1b]7501;") && bytes.ends_with(b"\x1b\\"));
        // Through the scanner and tap, exactly as the host sees it.
        let (mut t, s) = tap();
        feed(&mut t, &bytes);
        let r = rec(&s, &id).expect("record under tree id");
        assert_eq!(r.state, State::Blocked);
        assert_eq!(r.kind, Some(Kind::Question));
        assert_eq!(r.app.as_deref(), Some("plexi-agent"));
        assert_eq!(r.title.as_deref(), Some("Helper One ünï"));
        assert_eq!(r.msg.as_deref(), Some("which branch? please"));
        // And through the parser directly.
        let payload = &bytes[b"\x1b]7501;".len()..bytes.len() - 2];
        match parse_report(payload) {
            Some(Report::Set { id: got, record }) => {
                assert_eq!(got, id);
                assert_eq!(record.state, State::Blocked);
            }
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn ps7501_tree_id_sanitizes_and_limits() {
    use super::emit::tree_id;
    assert_eq!(tree_id(&["my agent:1", "ok"]).as_deref(), Some("my-agent-1/ok"));
    assert_eq!(tree_id(&["", "  "]), None);
    let long = "x".repeat(80);
    assert_eq!(tree_id(&[&long]).unwrap().len(), 32);
    let names: Vec<String> = (0..10).map(|i| format!("a{i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let id = tree_id(&refs).unwrap();
    assert_eq!(id.split('/').count(), 8);
    assert!(id.ends_with("a9"));
}

#[test]
fn ps7501_done_failure_detection() {
    use super::emit::done_is_failure;
    assert!(done_is_failure(r#"{"status":"failed"}"#));
    assert!(done_is_failure(r#"{"ok":false}"#));
    assert!(!done_is_failure(r#"{"status":"pass"}"#));
    assert!(!done_is_failure("not json"));
}
