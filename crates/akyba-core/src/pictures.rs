//! Pictures of a running app: its pages as a browser draws them, at a desktop's size and a phone's,
//! taken by a Chromium browser already on the Mac, invisibly (headless: no window, and the person's
//! own browser untouched). Each set is kept with its branch and time, so two can be laid side by side:
//! the default branch against an agent's, or before a turn against after it.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// The sizes pictures are taken at: a laptop's window, and a phone held upright.
pub const VIEWPORTS: [(&str, u32, u32); 2] = [("desktop", 1440, 900), ("mobile", 390, 844)];

/// A Chromium browser on this Mac, to draw pages with: Chrome, Chromium, Brave, Edge or Arc.
pub fn browser() -> Option<PathBuf> {
    [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        "/Applications/Arc.app/Contents/MacOS/Arc",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|path| path.is_file())
}

/// Draws `url` at `width`×`height` into the PNG at `out`. Blocks for up to half a minute: off the main
/// thread. The browser gets a profile of its own, so it never touches the person's.
pub fn capture(browser: &Path, url: &str, (width, height): (u32, u32), out: &Path) -> Result<(), String> {
    if let Some(folder) = out.parent() {
        std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    let profile = std::env::temp_dir().join(format!("akyba-pictures-{}", std::process::id()));
    let mut child = Command::new(browser)
        .args(["--headless=new", "--disable-gpu", "--hide-scrollbars", "--no-first-run", "--no-default-browser-check"])
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(format!("--window-size={width},{height}"))
        // Time for the page's scripts to draw it.
        .arg("--virtual-time-budget=8000")
        .arg(format!("--screenshot={}", out.display()))
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("The browser couldn't start: {error}"))?;
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > Duration::from_secs(30) => {
                let _ = child.kill();
                return Err(format!("{url} took too long to draw"));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(error) => return Err(error.to_string()),
        }
    }
    if out.is_file() { Ok(()) } else { Err(format!("The browser drew nothing for {url}")) }
}

/// The file name a page's picture is kept under: `desktop-cart-items.png` for `/cart/items`.
pub fn file_name(viewport: &str, route: &str) -> String {
    let route: String =
        route.trim_matches('/').chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let route = if route.is_empty() { "home".to_owned() } else { route };
    format!("{viewport}-{route}.png")
}

/// The pages someone typed, as paths: `/`, `cart, /orders?x=1` gives `/`, `/cart`, `/orders?x=1`.
pub fn routes(typed: &str) -> Vec<String> {
    let routes: Vec<String> = typed
        .split([',', ' ', '\n'])
        .map(str::trim)
        .filter(|route| !route.is_empty())
        .map(|route| if route.starts_with('/') { route.to_owned() } else { format!("/{route}") })
        .collect();
    if routes.is_empty() { vec!["/".to_owned()] } else { routes }
}

/// One set of pictures: where they are, what they show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Set {
    pub folder: PathBuf,
    /// The branch and the time, as the folder's name says: `fix-login · 2026-10-20 14:05`.
    pub label: String,
    /// Its pictures' file names, sorted.
    pub files: Vec<String>,
}

/// A new set's folder under `root`: named for when and from which branch, so they sort by time.
pub fn set_folder(root: &Path, branch: &str, now: &str) -> PathBuf {
    let branch: String = branch.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect();
    root.join(format!("{now} {branch}"))
}

/// The sets kept under `root`, newest first.
pub fn sets(root: &Path) -> Vec<Set> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut sets: Vec<Set> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let mut files: Vec<String> = std::fs::read_dir(entry.path())
                .ok()?
                .flatten()
                .map(|file| file.file_name().to_string_lossy().into_owned())
                .filter(|file| {
                    Path::new(file).extension().is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
                })
                .collect();
            files.sort();
            let (when, branch) = name.split_once(' ').unwrap_or((&name, ""));
            let label = format!("{branch} · {}", when.replacen('T', " ", 1));
            (!files.is_empty()).then(|| Set { folder: entry.path(), label, files })
        })
        .collect();
    sets.sort_by(|a, b| b.folder.cmp(&a.folder));
    sets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_pictures_and_reads_typed_pages() {
        assert_eq!(file_name("desktop", "/"), "desktop-home.png");
        assert_eq!(file_name("mobile", "/cart/items?x=1"), "mobile-cart-items-x-1.png");
        assert_eq!(routes(""), ["/"]);
        assert_eq!(routes("/, cart  /orders"), ["/", "/cart", "/orders"]);
    }

    #[test]
    fn keeps_sets_newest_first() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        for (when, branch) in [("2026-10-20T14.05", "main"), ("2026-10-20T15.30", "fix/login")] {
            let folder = set_folder(dir.path(), branch, when);
            std::fs::create_dir_all(&folder).expect("make folders");
            std::fs::write(folder.join("desktop-home.png"), b"png").expect("write");
        }
        std::fs::create_dir_all(dir.path().join("empty")).expect("make folders");
        let found = sets(dir.path());
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].label, "fix-login · 2026-10-20 15.30");
        assert_eq!(found[0].files, ["desktop-home.png"]);
    }

    #[test]
    fn draws_a_page_when_a_browser_is_here() {
        let Some(browser) = browser() else { return };
        let dir = tempfile::tempdir().expect("a temporary folder");
        let page = dir.path().join("page.html");
        std::fs::write(&page, "<h1 style='color:teal'>Cart</h1>").expect("write");
        let out = dir.path().join("shot.png");
        capture(&browser, &format!("file://{}", page.display()), (390, 844), &out).expect("drawn");
        let png = std::fs::read(&out).expect("a picture");
        assert!(png.starts_with(b"\x89PNG"));
    }
}
