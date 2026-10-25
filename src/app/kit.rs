//! The Vault kit: two printable, self-contained sheets. One carries the master
//! phrase, the other the ring secret as a QR code (inline SVG) and as text.
//! They are separate files so the two halves of the Vault secret can be kept
//! apart; neither names the other's content.

use std::io::Write;
use std::path::Path;

use qrcode::{Color, EcLevel, QrCode};

/// The sentence both sheets carry, and the warning.
pub const SHEET_IS_THE_STORE: &str = "This sheet is the credential store.";
pub const DO_NOT_PHOTOGRAPH: &str = "Do not photograph this sheet. A photo in a phone gallery or a cloud backup is a copy of the secret.";

/// What the ring-secret QR encodes: versioned text, so a scanner shows
/// something recognisable instead of 16 raw bytes.
pub fn ring_payload(ring_secret: &[u8; 16]) -> String {
    format!("sshub:ring:1:{}", hex::encode(ring_secret))
}

fn page(title: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\n<title>{title}</title>\n\
         <style>body{{font-family:sans-serif;max-width:40em;margin:2em auto}}\
         .warn{{border:3px solid #000;padding:1em;font-weight:bold}}\
         .secret{{font-family:monospace;font-size:1.4em;line-height:1.8}}\
         @media print{{body{{margin:1cm}}}}</style></head>\n<body>\n<h1>{title}</h1>\n\
         <p class=\"warn\">{DO_NOT_PHOTOGRAPH}</p>\n\
         <p><strong>{SHEET_IS_THE_STORE}</strong> Lose it and the credentials cannot be recovered; \
         let someone else read it and they can open them.</p>\n{body}</body></html>\n"
    )
}

/// The master phrase sheet: the six words, numbered.
pub fn phrase_sheet(phrase: &str) -> String {
    let words: String = phrase
        .split_whitespace()
        .map(|w| format!("<li>{w}</li>\n"))
        .collect();
    page(
        "sshub Vault: master phrase",
        &format!("<ol class=\"secret\">\n{words}</ol>\n<p>Six words, in this order.</p>\n"),
    )
}

/// The paper phrase sheet: the 24 words, numbered.
pub fn paper_sheet(words: &str) -> String {
    let words: String = words
        .split_whitespace()
        .map(|w| format!("<li>{w}</li>\n"))
        .collect();
    page(
        "sshub: paper phrase",
        &format!("<ol class=\"secret\">\n{words}</ol>\n<p>Twenty-four words, in this order.</p>\n"),
    )
}

/// The QR of `payload` as an inline SVG: black modules on white with a
/// 4-module quiet zone, one `path` of unit squares.
pub fn qr_svg(payload: &str) -> String {
    let code = QrCode::with_error_correction_level(payload, EcLevel::M)
        .expect("a ring secret payload always fits a QR code");
    let n = code.width();
    let side = n + 8;
    let mut path = String::new();
    for (i, c) in code.to_colors().iter().enumerate() {
        if *c == Color::Dark {
            path.push_str(&format!("M{},{}h1v1h-1z", i % n + 4, i / n + 4));
        }
    }
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {side} {side}\" width=\"320\" height=\"320\" \
         shape-rendering=\"crispEdges\"><rect width=\"{side}\" height=\"{side}\" fill=\"#fff\"/>\
         <path fill=\"#000\" d=\"{path}\"/></svg>"
    )
}

/// The ring secret sheet: the QR, and the same secret as text for a camera
/// that will not read it.
pub fn ring_sheet(ring_secret: &[u8; 16]) -> String {
    let payload = ring_payload(ring_secret);
    let hex = hex::encode(ring_secret);
    let grouped: Vec<&str> = (0..hex.len()).step_by(4).map(|i| &hex[i..i + 4]).collect();
    page(
        "sshub Vault: ring secret",
        &format!(
            "<div>{}</div>\n<p class=\"secret\">{}</p>\n\
             <p>Scan it when adding a device, or type the groups above.</p>\n",
            qr_svg(&payload),
            grouped.join(" ")
        ),
    )
}

/// Where the sheets go by default: the runtime directory (memory-backed, owner-only,
/// gone at logout) when there is one, else the home directory.
pub fn default_dir() -> std::path::PathBuf {
    default_dir_from(
        std::env::var_os("XDG_RUNTIME_DIR"),
        std::env::var_os("HOME"),
    )
}

fn default_dir_from(
    runtime: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> std::path::PathBuf {
    runtime
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(|| home.map(std::path::PathBuf::from))
        .unwrap_or_else(|| ".".into())
}

/// Sheets this process wrote (the kit, the paper phrase): the quit reminder lists them.
static WRITTEN: std::sync::Mutex<Vec<std::path::PathBuf>> = std::sync::Mutex::new(Vec::new());

/// A file with secrets this run wrote by its own means (the export CSV): the quit reminder lists it.
pub fn note_written(path: &Path) {
    WRITTEN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(path.to_path_buf());
}

/// The sheets written in this run that are still on disk.
pub fn sheets_left() -> Vec<std::path::PathBuf> {
    let written = WRITTEN.lock().unwrap_or_else(|e| e.into_inner());
    written.iter().filter(|p| p.exists()).cloned().collect()
}

/// After the terminal is restored: say which sheets are still on disk. They are the
/// credential store; move or print them, then delete them.
pub fn print_reminder() {
    let left = sheets_left();
    if left.is_empty() {
        return;
    }
    eprintln!("sshub: these sheets hold your secrets and are still on disk:");
    for p in left {
        eprintln!("  {}", p.display());
    }
    eprintln!("sshub: move or print them, then delete them.");
}

/// Write a sheet that does not exist yet, owner-only from the first byte.
pub fn write_sheet(path: &Path, html: &str) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(html.as_bytes())?;
    file.flush()?;
    note_written(path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sheets_default_into_the_runtime_directory_and_the_quit_reminder_lists_them() {
        let run = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        assert_eq!(
            default_dir_from(Some(run.path().into()), Some(home.path().into())),
            run.path()
        );
        // No runtime directory (unset, or not a directory): the home directory.
        assert_eq!(
            default_dir_from(None, Some(home.path().into())),
            home.path()
        );
        assert_eq!(
            default_dir_from(Some("/nonexistent/run".into()), Some(home.path().into())),
            home.path()
        );
        // A written sheet is listed until it is deleted.
        let sheet = run.path().join("sheet.html");
        write_sheet(&sheet, "x").unwrap();
        assert!(sheets_left().contains(&sheet));
        std::fs::remove_file(&sheet).unwrap();
        assert!(!sheets_left().contains(&sheet));
    }

    /// The module matrix drawn by [`qr_svg`], read back from its path.
    fn svg_matrix(svg: &str) -> (usize, Vec<bool>) {
        let side: usize = svg
            .split("viewBox=\"0 0 ")
            .nth(1)
            .and_then(|s| s.split(' ').next())
            .unwrap()
            .parse()
            .unwrap();
        let d = svg
            .split("d=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap();
        let mut grid = vec![false; side * side];
        for cmd in d.split('M').filter(|c| !c.is_empty()) {
            let (xy, _) = cmd.split_once('h').unwrap();
            let (x, y) = xy.split_once(',').unwrap();
            grid[y.parse::<usize>().unwrap() * side + x.parse::<usize>().unwrap()] = true;
        }
        (side, grid)
    }

    #[test]
    fn the_svg_is_the_qr_of_the_payload_with_a_quiet_zone() {
        let ring = [0x5a; 16];
        let svg = qr_svg(&ring_payload(&ring));
        let (side, grid) = svg_matrix(&svg);
        let want = QrCode::with_error_correction_level(ring_payload(&ring), EcLevel::M).unwrap();
        let n = want.width();
        assert_eq!(side, n + 8);
        for y in 0..side {
            for x in 0..side {
                let inside = (4..4 + n).contains(&x) && (4..4 + n).contains(&y);
                let expect = inside && want.to_colors()[(y - 4) * n + (x - 4)] == Color::Dark;
                assert_eq!(grid[y * side + x], expect, "module ({x},{y})");
            }
        }
    }

    #[test]
    fn each_sheet_carries_the_warning_and_only_its_own_secret() {
        let ring = [0x11; 16];
        let phrase = "abacus abdomen abdominal abide abiding ability";
        let (p, r) = (phrase_sheet(phrase), ring_sheet(&ring));
        for sheet in [&p, &r] {
            assert!(sheet.contains(DO_NOT_PHOTOGRAPH) && sheet.contains(SHEET_IS_THE_STORE));
            assert!(sheet.contains("<!DOCTYPE html>") && !sheet.contains("src=\"http"));
        }
        assert!(phrase
            .split(' ')
            .all(|w| p.contains(&format!("<li>{w}</li>"))));
        assert!(!r.contains("abacus") && !p.contains(&hex::encode(ring)));
        assert!(r.contains("<svg") && r.contains(&hex::encode(ring)[..4]));
    }

    #[test]
    fn a_sheet_is_created_owner_only_and_never_overwritten() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sheet.html");
        write_sheet(&path, "<html>x</html>").unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(
            write_sheet(&path, "other").is_err(),
            "an existing file is not overwritten"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "<html>x</html>");
    }
}
