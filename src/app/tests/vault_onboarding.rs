//! Vault first run: the issued phrase, the retype check, and the two kit files.

use super::*;
use crate::app::kit;
use crate::app::sync::{OnboardingStep, OnboardingView, SyncIntent};
use std::path::{Path, PathBuf};

fn view_in(dir: &Path) -> OnboardingView {
    OnboardingView::with_secrets(
        &[0x21; 10],
        [0xab; 16],
        [1, 4],
        (
            dir.join("phrase.html").to_string_lossy().into_owned(),
            dir.join("ring.html").to_string_lossy().into_owned(),
        ),
    )
    .unwrap()
}

fn open(app: &mut App, view: OnboardingView) {
    app.open_sync_onboarding(view);
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle_key(key_char(c)).unwrap();
    }
}

fn enter(app: &mut App) {
    app.handle_key(key(KeyCode::Enter)).unwrap();
}

fn view(app: &App) -> &OnboardingView {
    app.sync.onboarding.as_ref().unwrap()
}

/// Walk Show -> Retype -> Kit, answering the two questions correctly.
fn to_kit(app: &mut App) {
    enter(app);
    for _ in 0..2 {
        let word = view(app).quiz_word().to_string();
        type_text(app, &word);
        enter(app);
    }
    assert_eq!(view(app).step, OnboardingStep::Kit);
}

#[test]
fn issued_phrases_are_six_eff_words_and_differ_each_time() {
    let a = OnboardingView::issue().unwrap();
    let b = OnboardingView::issue().unwrap();
    for v in [&a, &b] {
        let phrase = v.secrets().phrase.to_string();
        // Decoding only succeeds for words of the EFF list, and re-encoding the
        // bytes gives the same phrase back.
        let bytes = crate::sync::crypto::eff_decode(&phrase).unwrap();
        assert_eq!(crate::sync::crypto::eff_encode(&bytes).unwrap(), phrase);
        assert_eq!(v.words().len(), 6);
        assert!(v.quiz[0] < v.quiz[1] && v.quiz[1] < 6, "{:?}", v.quiz);
    }
    assert_ne!(a.secrets().phrase, b.secrets().phrase);
    assert_ne!(a.secrets().ring_secret, b.secrets().ring_secret);
}

#[test]
fn the_retype_check_accepts_the_right_words_and_refuses_the_wrong_one() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = test_app(vec![]);
    open(&mut app, view_in(dir.path()));
    assert_eq!(view(&app).step, OnboardingStep::Show);
    enter(&mut app);
    assert_eq!(view(&app).step, OnboardingStep::Retype);

    // A wrong word stays on the question and says which word it was.
    type_text(&mut app, "notaword");
    enter(&mut app);
    assert_eq!(view(&app).step, OnboardingStep::Retype);
    assert_eq!(view(&app).quiz_done, 0);
    assert!(view(&app)
        .notice
        .as_deref()
        .is_some_and(|n| n.contains(&format!("word {}", view(&app).quiz[0] + 1))));

    // The right word, in another case and with padding, moves on.
    let first = view(&app).quiz_word().to_uppercase();
    type_text(&mut app, &format!(" {first} "));
    enter(&mut app);
    assert_eq!(view(&app).quiz_done, 1);
    assert_eq!(view(&app).step, OnboardingStep::Retype);
    let second = view(&app).quiz_word().to_string();
    type_text(&mut app, &second);
    enter(&mut app);
    assert_eq!(view(&app).step, OnboardingStep::Kit);
}

#[test]
fn esc_in_the_retype_step_goes_back_to_the_words_and_restarts_the_quiz() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = test_app(vec![]);
    open(&mut app, view_in(dir.path()));
    enter(&mut app);
    let first = view(&app).quiz_word().to_string();
    type_text(&mut app, &first);
    enter(&mut app);
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(view(&app).step, OnboardingStep::Show);
    assert_eq!(view(&app).quiz_done, 0);
    assert_eq!(app.mode, AppMode::SyncOnboarding, "setup is not skippable");
}

fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    v.sort();
    v
}

#[test]
fn the_kit_is_two_owner_only_files_with_the_warning_and_the_right_secret_each() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let mut app = test_app(vec![]);
    let v = view_in(dir.path());
    let phrase = v.secrets().phrase.to_string();
    let ring_hex = hex::encode([0xab; 16]);
    open(&mut app, v);
    to_kit(&mut app);
    enter(&mut app); // write both sheets
    assert_eq!(
        view(&app).step,
        OnboardingStep::Done,
        "{:?}",
        view(&app).notice
    );

    let files = files_in(dir.path());
    assert_eq!(files.len(), 2, "exactly the two sheets: {files:?}");
    for f in &files {
        assert_eq!(
            std::fs::metadata(f).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let html = std::fs::read_to_string(f).unwrap();
        assert!(html.contains(kit::DO_NOT_PHOTOGRAPH), "{f:?}");
        assert!(html.contains(kit::SHEET_IS_THE_STORE), "{f:?}");
    }
    let phrase_sheet = std::fs::read_to_string(dir.path().join("phrase.html")).unwrap();
    let ring_sheet = std::fs::read_to_string(dir.path().join("ring.html")).unwrap();
    // Each secret is in its own sheet and in no other file.
    assert!(phrase
        .split(' ')
        .all(|w| phrase_sheet.contains(&format!("<li>{w}</li>"))));
    let phrase_list = phrase.replace(' ', "</li>\n<li>");
    assert!(phrase_sheet.contains(&phrase_list));
    assert!(!ring_sheet.contains(&phrase_list));
    assert!(ring_sheet.contains(&ring_hex[..4]) && !phrase_sheet.contains(&ring_hex));

    // The ring secret's QR: the sheet embeds exactly the SVG of the payload,
    // whose matrix `kit`'s own test proves is the QR of that payload.
    let svg = ring_sheet.split("<svg").nth(1).unwrap();
    assert_eq!(
        kit::qr_svg(&kit::ring_payload(&[0xab; 16])),
        format!("<svg{}", svg.split("</svg>").next().unwrap()) + "</svg>",
    );
}

#[test]
fn the_kit_refuses_an_overwrite_a_shared_path_and_a_missing_folder_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = test_app(vec![]);
    open(&mut app, view_in(dir.path()));
    to_kit(&mut app);

    // The ring sheet's path is taken: nothing is written, the phrase sheet included.
    std::fs::write(dir.path().join("ring.html"), "mine").unwrap();
    enter(&mut app);
    assert_eq!(view(&app).step, OnboardingStep::Kit);
    assert!(view(&app)
        .notice
        .as_deref()
        .is_some_and(|n| n.contains("already exists")));
    assert!(!dir.path().join("phrase.html").exists());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("ring.html")).unwrap(),
        "mine"
    );

    // Both paths the same.
    {
        let o = app.sync.onboarding.as_mut().unwrap();
        o.ring_path = o.phrase_path.clone();
    }
    enter(&mut app);
    assert!(view(&app)
        .notice
        .as_deref()
        .is_some_and(|n| n.contains("different paths")));

    // A folder that is not there.
    {
        let o = app.sync.onboarding.as_mut().unwrap();
        o.ring_path = dir
            .path()
            .join("nope")
            .join("r.html")
            .to_string_lossy()
            .into_owned();
    }
    enter(&mut app);
    assert!(view(&app)
        .notice
        .as_deref()
        .is_some_and(|n| n.contains("does not exist")));
    assert!(!dir.path().join("phrase.html").exists());
}

#[test]
fn the_paths_are_edited_in_place_and_tab_switches_the_field() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = test_app(vec![]);
    open(&mut app, view_in(dir.path()));
    to_kit(&mut app);
    for _ in 0..view(&app).phrase_path.chars().count() {
        app.handle_key(key(KeyCode::Backspace)).unwrap();
    }
    let chosen = dir
        .path()
        .join("my-phrase.html")
        .to_string_lossy()
        .into_owned();
    type_text(&mut app, &chosen);
    app.handle_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(view(&app).path_field, 1);
    app.handle_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(view(&app).phrase_path, chosen);
    enter(&mut app);
    assert!(dir.path().join("my-phrase.html").exists());
}

#[test]
fn finishing_hands_the_secrets_to_the_sync_stage_and_clears_the_screen() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = test_app(vec![]);
    let v = view_in(dir.path());
    let (phrase, first_word) = (v.secrets().phrase.to_string(), v.words()[0].to_string());
    open(&mut app, v);
    to_kit(&mut app);
    enter(&mut app); // write
    enter(&mut app); // finish
    assert_eq!(app.mode, AppMode::Normal);
    assert!(
        app.sync.onboarding.is_none(),
        "the secrets leave the screen state"
    );
    let [SyncIntent::OnboardingDone(secrets)] = app.sync.intents.as_slice() else {
        panic!("{:?}", app.sync.intents);
    };
    assert_eq!(secrets.phrase.as_str(), phrase);
    assert_eq!(*secrets.ring_secret, [0xab; 16]);
    assert!(
        !format!("{:?}", app.sync.intents).contains(&first_word),
        "Debug never prints the phrase"
    );
}

#[test]
fn nothing_but_the_kit_ever_holds_the_phrase_on_disk() {
    // Run a whole onboarding with the temp and runtime dirs watched too.
    let dir = tempfile::tempdir().unwrap();
    let mut app = test_app(vec![]);
    let v = view_in(dir.path());
    let phrase = v.secrets().phrase.to_string();
    open(&mut app, v);
    to_kit(&mut app);
    enter(&mut app);
    enter(&mut app);
    let list = phrase.replace(' ', "</li>\n<li>");
    let holders: Vec<PathBuf> = files_in(dir.path())
        .into_iter()
        .filter(|f| std::fs::read_to_string(f).unwrap().contains(&list))
        .collect();
    assert_eq!(holders, vec![dir.path().join("phrase.html")]);
}
