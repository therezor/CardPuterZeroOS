//! Files opened lately, in the shared list freedesktop apps keep:
//! `~/.local/share/recently-used.xbel`. Our apps add to it; GTK apps on the
//! device read and add to the same one. Other apps' entries are kept as
//! they were written. The newest 100 stay. Settings turns it off
//! (`gh0st.recent.remember`) and clears it.

use std::path::{Path, PathBuf};

/// Entries kept, newest first.
pub const KEEP: usize = 100;
/// A list larger than this is someone else's to manage: read none of it
/// and write nothing over it.
const MAX_FILE: u64 = 1 << 20;

pub fn path(home: &Path) -> PathBuf {
    home.join(".local/share/recently-used.xbel")
}

/// One `<bookmark>` as written, with what we read from it.
struct Mark {
    href: String,
    modified: String,
    raw: String,
}

/// The value of attribute `name` in the tag `tag`.
fn attr(tag: &str, name: &str) -> Option<String> {
    let at = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    let end = tag[at..].find('"')?;
    Some(unescape(&tag[at..at + end]))
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn read_marks(file: &Path) -> Option<Vec<Mark>> {
    match std::fs::metadata(file) {
        Ok(m) if m.len() > MAX_FILE => return None,
        Ok(_) => {}
        Err(_) => return Some(Vec::new()),
    }
    let text = std::fs::read_to_string(file).ok()?;
    let mut marks = Vec::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find("<bookmark ") {
        let after = &rest[start..];
        let end = match (after.find("</bookmark>"), after.find("/>")) {
            // A bookmark with nothing inside closes itself.
            (_, Some(s)) if after[..s].find('>').is_none() => s + 2,
            (Some(e), _) => e + "</bookmark>".len(),
            _ => break,
        };
        let raw = &after[..end];
        let head = &raw[..raw.find('>').unwrap_or(raw.len())];
        if let Some(href) = attr(head, "href") {
            let modified = attr(head, "modified").unwrap_or_default();
            marks.push(Mark {
                href,
                modified,
                raw: raw.to_string(),
            });
        }
        rest = &after[end..];
    }
    Some(marks)
}

/// The local files in the list, newest first, at most `n`.
pub fn list(home: &Path, n: usize) -> Vec<PathBuf> {
    let mut marks = read_marks(&path(home)).unwrap_or_default();
    marks.sort_by(|a, b| b.modified.cmp(&a.modified));
    marks
        .iter()
        .filter_map(|m| crate::uri::to_path(&m.href))
        .take(n)
        .collect()
}

/// Now, as the list writes times: UTC, ISO 8601.
fn now_iso() -> String {
    let (secs, micros) = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_secs() as libc::time_t, d.subsec_micros()))
        .unwrap_or((0, 0));
    // SAFETY: gmtime_r into a local struct tm.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::gmtime_r(&secs, &mut tm);
        tm
    };
    format!(
        "{}-{:02}-{:02}T{:02}:{:02}:{:02}.{micros:06}Z",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// Adds `file`, opened by `app` (its name, and the command line that
/// opens it), or moves it to the top.
pub fn add(home: &Path, file: &Path, mime: &str, app: &str, exec: &str) -> std::io::Result<()> {
    let list = path(home);
    let Some(mut marks) = read_marks(&list) else {
        return Ok(());
    };
    let href = crate::uri::from_path(file);
    let now = now_iso();
    let added = marks
        .iter()
        .find(|m| m.href == href)
        .and_then(|m| {
            let head = &m.raw[..m.raw.find('>').unwrap_or(m.raw.len())];
            attr(head, "added")
        })
        .unwrap_or_else(|| now.clone());
    marks.retain(|m| m.href != href);
    let raw = format!(
        "<bookmark href=\"{}\" added=\"{}\" modified=\"{now}\" visited=\"{now}\">\n\
         \x20   <info>\n\
         \x20     <metadata owner=\"http://freedesktop.org\">\n\
         \x20       <mime:mime-type type=\"{}\"/>\n\
         \x20       <bookmark:applications>\n\
         \x20         <bookmark:application name=\"{}\" exec=\"{}\" modified=\"{now}\" count=\"1\"/>\n\
         \x20       </bookmark:applications>\n\
         \x20     </metadata>\n\
         \x20   </info>\n\
         \x20 </bookmark>",
        escape(&href),
        escape(&added),
        escape(mime),
        escape(app),
        escape(exec),
    );
    marks.push(Mark {
        href,
        modified: now,
        raw,
    });
    marks.sort_by(|a, b| b.modified.cmp(&a.modified));
    marks.truncate(KEEP);
    write(&list, &marks)
}

/// Every entry goes, ours and other apps'.
pub fn clear(home: &Path) -> std::io::Result<()> {
    let list = path(home);
    if !list.exists() {
        return Ok(());
    }
    write(&list, &[])
}

fn write(list: &Path, marks: &[Mark]) -> std::io::Result<()> {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <xbel version=\"1.0\"\n\
         \x20     xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\"\n\
         \x20     xmlns:mime=\"http://www.freedesktop.org/standards/shared-mime-info\"\n>\n",
    );
    for m in marks.iter().rev() {
        out.push_str("  ");
        out.push_str(&m.raw);
        out.push('\n');
    }
    out.push_str("</xbel>\n");
    if let Some(d) = list.parent() {
        std::fs::create_dir_all(d)?;
    }
    crate::config::write_atomic(list, out.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_others_kept_and_a_file_moves_to_the_top() {
        let home = std::env::temp_dir().join(format!("gh0st-recent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".local/share")).unwrap();
        // A GTK app's entry, with its own metadata.
        std::fs::write(
            path(&home),
            "<?xml version=\"1.0\"?>\n<xbel version=\"1.0\">\n  <bookmark href=\"file:///srv/x.odt\" \
             added=\"2020-01-01T00:00:00Z\" modified=\"2020-01-01T00:00:00Z\" visited=\"2020-01-01T00:00:00Z\">\
             <info><metadata owner=\"http://freedesktop.org\"><mime:mime-type type=\"x/y\"/></metadata></info>\
             </bookmark>\n</xbel>\n",
        )
        .unwrap();
        let a = home.join("a & b.txt");
        add(&home, &a, "text/plain", "Text", "gh0st-text %u").unwrap();
        add(
            &home,
            &home.join("c.png"),
            "image/png",
            "Media",
            "gh0st-media %u",
        )
        .unwrap();
        add(&home, &a, "text/plain", "Text", "gh0st-text %u").unwrap();
        let l = list(&home, 10);
        assert_eq!(l[0], a, "opened again: the top");
        assert_eq!(l[1], home.join("c.png"));
        assert_eq!(l[2], PathBuf::from("/srv/x.odt"), "another app's stays");
        let text = std::fs::read_to_string(path(&home)).unwrap();
        assert!(
            text.contains("<mime:mime-type type=\"x/y\"/>"),
            "kept as written"
        );
        assert_eq!(text.matches("a%20%26%20b.txt").count(), 1, "once");
        clear(&home).unwrap();
        assert!(list(&home, 10).is_empty());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn only_the_newest_hundred_stay() {
        let home = std::env::temp_dir().join(format!("gh0st-recent-many-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for i in 0..KEEP + 5 {
            add(
                &home,
                &home.join(format!("{i}.txt")),
                "text/plain",
                "Text",
                "t",
            )
            .unwrap();
        }
        let l = list(&home, 1000);
        assert_eq!(l.len(), KEEP);
        assert_eq!(l[0], home.join(format!("{}.txt", KEEP + 4)));
        std::fs::remove_dir_all(&home).unwrap();
    }
}
