//! Config sharing: live links, conflicts, unsharing, and deletion safety.

use keepapart::profile::{LinkMode, Store, Tool};
use keepapart::share::{self, lookup, ItemState, ShareOutcome};
use std::fs;

fn setup() -> (tempfile::TempDir, Store) {
    let t = tempfile::tempdir().unwrap();
    let s = Store::at(t.path().join("kp")).unwrap();
    (t, s)
}

#[test]
fn shared_folder_is_live_across_profiles() {
    let (_t, s) = setup();
    let mut a = s.create("work", &[Tool::Claude]).unwrap();
    let mut b = s.create("personal", &[Tool::Claude]).unwrap();
    let skills = lookup("claude:skills").unwrap();
    share::share(&s, &mut a, skills, false).unwrap();
    share::share(&s, &mut b, skills, false).unwrap();

    fs::create_dir_all(a.tool_dir(Tool::Claude).join("skills").join("review")).unwrap();
    fs::write(a.tool_dir(Tool::Claude).join("skills").join("review").join("SKILL.md"), "hi").unwrap();
    assert_eq!(
        fs::read_to_string(b.tool_dir(Tool::Claude).join("skills").join("review").join("SKILL.md")).unwrap(),
        "hi"
    );
    assert!(matches!(share::state(&s, &a, skills), ItemState::Shared(LinkMode::Symlink | LinkMode::Junction)));
}

#[test]
fn first_sharer_seeds_the_store_and_conflicts_need_force() {
    let (_t, s) = setup();
    let mut a = s.create("a", &[Tool::Claude]).unwrap();
    let mut b = s.create("b", &[Tool::Claude]).unwrap();
    let md = lookup("claude:CLAUDE.md").unwrap();
    fs::write(a.tool_dir(Tool::Claude).join("CLAUDE.md"), "from a").unwrap();
    fs::write(b.tool_dir(Tool::Claude).join("CLAUDE.md"), "from b").unwrap();

    assert!(matches!(share::share(&s, &mut a, md, false).unwrap(), ShareOutcome::Seeded(_)));
    assert_eq!(fs::read_to_string(s.shared_dir(Tool::Claude).join("CLAUDE.md")).unwrap(), "from a");

    // b has its own different copy: refuse without --force, keep b untouched
    assert!(share::share(&s, &mut b, md, false).is_err());
    assert_eq!(fs::read_to_string(b.tool_dir(Tool::Claude).join("CLAUDE.md")).unwrap(), "from b");

    match share::share(&s, &mut b, md, true).unwrap() {
        ShareOutcome::BackedUp(_, backup) => assert_eq!(fs::read_to_string(backup).unwrap(), "from b"),
        o => panic!("expected backup, got {o:?}"),
    }
    assert_eq!(fs::read_to_string(b.tool_dir(Tool::Claude).join("CLAUDE.md")).unwrap(), "from a");
    // sharing again is a no-op
    assert!(matches!(share::share(&s, &mut b, md, false).unwrap(), ShareOutcome::AlreadyShared(_)));
}

#[test]
fn new_json_items_start_valid() {
    let (_t, s) = setup();
    let mut a = s.create("a", &[Tool::Claude]).unwrap();
    share::share(&s, &mut a, lookup("claude:settings.json").unwrap(), false).unwrap();
    let text = fs::read_to_string(a.tool_dir(Tool::Claude).join("settings.json")).unwrap();
    serde_json::from_str::<serde_json::Value>(&text).unwrap();
}

#[test]
fn unshare_leaves_a_private_copy_and_the_store_alone() {
    let (_t, s) = setup();
    let mut a = s.create("a", &[Tool::Codex]).unwrap();
    let mut b = s.create("b", &[Tool::Codex]).unwrap();
    let prompts = lookup("codex:prompts").unwrap();
    share::share(&s, &mut a, prompts, false).unwrap();
    share::share(&s, &mut b, prompts, false).unwrap();
    fs::write(a.tool_dir(Tool::Codex).join("prompts").join("p.md"), "v1").unwrap();

    share::unshare(&s, &mut a, prompts).unwrap();
    let mine = a.tool_dir(Tool::Codex).join("prompts");
    assert!(!share::is_link(&mine));
    assert_eq!(fs::read_to_string(mine.join("p.md")).unwrap(), "v1");
    fs::write(mine.join("p.md"), "v2 private").unwrap();
    assert_eq!(fs::read_to_string(b.tool_dir(Tool::Codex).join("prompts").join("p.md")).unwrap(), "v1");
    assert_eq!(share::state(&s, &a, prompts), ItemState::Private);
    assert!(share::unshare(&s, &mut a, prompts).is_err());
}

#[test]
fn removing_a_profile_never_deletes_shared_content() {
    let (_t, s) = setup();
    let mut a = s.create("a", &[Tool::Claude, Tool::Codex]).unwrap();
    let mut b = s.create("b", &[Tool::Claude]).unwrap();
    share::apply_defaults(&s, &mut a).unwrap();
    share::apply_defaults(&s, &mut b).unwrap();
    fs::write(a.tool_dir(Tool::Claude).join("agents").join("x.md"), "agent").unwrap();
    fs::write(a.tool_dir(Tool::Claude).join(".credentials.json"), "{}").unwrap();

    s.remove("a").unwrap();
    assert!(!a.dir.exists());
    assert_eq!(fs::read_to_string(s.shared_dir(Tool::Claude).join("agents").join("x.md")).unwrap(), "agent");
    assert_eq!(fs::read_to_string(b.tool_dir(Tool::Claude).join("agents").join("x.md")).unwrap(), "agent");
}

#[test]
fn sharing_needs_the_tool_and_refuses_foreign_links() {
    let (t, s) = setup();
    let mut a = s.create("a", &[Tool::Claude]).unwrap();
    assert!(share::share(&s, &mut a, lookup("codex:prompts").unwrap(), false).is_err());

    // a link someone made by hand to another place is left alone
    let elsewhere = t.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    let link = a.tool_dir(Tool::Claude).join("commands");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&elsewhere, &link).unwrap();
    #[cfg(windows)]
    if std::os::windows::fs::symlink_dir(&elsewhere, &link).is_err() {
        junction::create(&elsewhere, &link).unwrap();
    }
    assert!(share::share(&s, &mut a, lookup("claude:commands").unwrap(), false).is_err());
    assert!(elsewhere.exists());
}

#[test]
fn defaults_share_only_default_items_for_the_profiles_tools() {
    let (_t, s) = setup();
    let mut a = s.create("a", &[Tool::Claude]).unwrap();
    let res = share::apply_defaults(&s, &mut a).unwrap();
    assert!(res.iter().all(|(_, r)| r.is_ok()));
    let ids: Vec<&str> = a.file.shared.iter().map(|e| e.item.as_str()).collect();
    assert!(ids.contains(&"claude:skills"));
    assert!(ids.contains(&"claude:CLAUDE.md"));
    assert!(!ids.contains(&"claude:settings.json"));
    assert!(!ids.iter().any(|i| i.starts_with("codex:")));
}
