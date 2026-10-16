//! Job folders (Qt app/jobs.*): a job is a folder under the jobs root holding `.tlp` projects, `job.json`
//! (status/notes), `gcode/` with the current G-code and `gcode/history/` with earlier revisions
//! (`<file>.r<N>.<stamp>.<ext>`). Exported file names are prefixed with the project stem. Desktop only (file system).
#![cfg(not(target_arch = "wasm32"))]

use gcarve_core::{pipeline, project};
use std::path::{Path, PathBuf};

const GCODE_EXTS: [&str; 9] = ["nc", "tap", "gcode", "ngc", "txt", "cnc", "gc", "mmg", "sbp"];
pub const STATUSES: [&str; 4] = ["Planned", "In progress", "Cut", "Delivered"];

#[derive(Clone, Debug, Default)]
pub struct ProjectInfo {
    pub name: String,
    pub path: PathBuf,
    pub modified: String,
    pub exported: String,
    pub files: usize,
    pub revision: i32,
    pub stale: bool,
}

#[derive(Clone, Debug, Default)]
pub struct JobInfo {
    pub name: String,
    pub path: PathBuf,
    pub status: String,
    pub notes: String,
    pub projects: Vec<ProjectInfo>,
}

#[derive(Clone, Debug)]
pub struct Revision {
    pub name: String,
    pub path: PathBuf,
    pub rev: i32,
    pub time: String,
}

pub struct Jobs {
    pub root: PathBuf,
    pub jobs: Vec<JobInfo>,
    pub status: String,
}

fn fmt_time(t: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(t).format("%-d %b %Y, %-I:%M %p").to_string()
}

fn read_meta(job: &Path) -> serde_json::Value {
    std::fs::read_to_string(job.join("job.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(serde_json::json!({}))
}
fn write_meta(job: &Path, v: &serde_json::Value) {
    let _ = std::fs::write(job.join("job.json"), serde_json::to_string_pretty(v).unwrap_or_default());
}

/// revision number in "<stem>.r<N>.<stamp>.<ext>"
pub fn revision_of(name: &str) -> i32 {
    name.split(".r").nth(1).and_then(|s| s.split('.').next()).and_then(|n| n.parse().ok()).unwrap_or(0)
}

/// the post header carries the export time: ignore it when deciding whether the G-code actually changed
/// ponytail: drops whole lines holding a "hh:mm AM/PM, weekday month day year" stamp instead of a regex
fn without_timestamps(text: &str) -> String {
    text.lines().filter(|l| !((l.contains(" AM, ") || l.contains(" PM, ")) && l.split_whitespace().any(|y| y.len() == 4 && y.chars().all(|c| c.is_ascii_digit())))).collect::<Vec<_>>().join("\n")
}

/// Archive the file at `path` (if present and different from `text`) into `history/` next to it; returns the
/// revision number the new file will have.
pub fn archive_revision(path: &Path, text: &str) -> i32 {
    let dir = path.parent().unwrap_or(Path::new("."));
    let hist = dir.join("history");
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let prev = std::fs::read_dir(&hist).map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with(&format!("{stem}.r"))).map(|n| revision_of(&n)).max().unwrap_or(0)).unwrap_or(0);
    let Ok(old) = std::fs::read_to_string(path) else { return prev + 1 };
    if without_timestamps(&old) == without_timestamps(text) {
        return prev + 1; // identical: nothing to archive, same revision
    }
    let _ = std::fs::create_dir_all(&hist);
    let stamp = std::fs::metadata(path).and_then(|m| m.modified()).map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%Y%m%d-%H%M").to_string()).unwrap_or_default();
    let _ = std::fs::rename(path, hist.join(format!("{stem}.r{}.{stamp}{ext}", prev + 1)));
    prev + 2
}

impl Jobs {
    /// Screenshot sample (Qt `--dialog jobs`): a fresh root in the temp dir with one job holding the showcase
    /// project, exported once. Never persisted as the user's root.
    pub fn sample() -> Self {
        let root = std::env::temp_dir().join("gcarve-shot-jobs");
        let _ = std::fs::remove_dir_all(&root);
        let mut j = Jobs { root, jobs: Vec::new(), status: String::new() };
        if let Some(job) = j.create("Smith sign") {
            let src = std::env::args().skip(1).find(|a| a.ends_with(".tlp")).map(PathBuf::from);
            if let Some(p) = src.and_then(|s| j.add_project(&job, &s)) {
                j.export_project(&p, &crate::posts::UserPosts::default());
            }
        }
        j
    }

    pub fn load() -> Self {
        let root = crate::store::load("jobs-root.txt").map(|s| PathBuf::from(s.trim())).filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| std::env::var("HOME").map(|h| PathBuf::from(h).join("Documents/GCarve Jobs")).unwrap_or_else(|_| PathBuf::from("GCarve Jobs")));
        let mut j = Jobs { root, jobs: Vec::new(), status: String::new() };
        j.refresh();
        j
    }
    pub fn set_root(&mut self, dir: PathBuf) {
        self.root = dir;
        crate::store::save("jobs-root.txt", &self.root.to_string_lossy());
        self.refresh();
    }

    /// the job folder a project sits in (direct child of the root), if any
    pub fn job_for(&self, project: &Path) -> Option<PathBuf> {
        let rel = project.strip_prefix(&self.root).ok()?;
        let first = rel.components().next()?;
        (rel.components().count() >= 2).then(|| self.root.join(first))
    }

    fn gcode_for(job: &Path, stem: &str) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(job.join("gcode")).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && p.extension().is_some_and(|e| GCODE_EXTS.contains(&e.to_string_lossy().to_ascii_lowercase().as_str())) && p.file_stem().is_some_and(|s| s.to_string_lossy().starts_with(&format!("{stem}_")))).collect()).unwrap_or_default();
        v.sort();
        v
    }

    pub fn revisions(&self, project: &Path) -> Vec<Revision> {
        let Some(job) = self.job_for(project) else { return Vec::new() };
        let stem = project.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut out: Vec<(std::time::SystemTime, Revision)> = std::fs::read_dir(job.join("gcode/history")).map(|rd| rd.flatten().filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.starts_with(&format!("{stem}_")) {
                return None;
            }
            let t = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
            Some((t, Revision { rev: revision_of(&name), time: fmt_time(t), path: e.path(), name }))
        }).collect()).unwrap_or_default();
        out.sort_by_key(|a| std::cmp::Reverse(a.0));
        out.into_iter().map(|(_, r)| r).collect()
    }

    pub fn refresh(&mut self) {
        self.jobs.clear();
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(&self.root).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default();
        dirs.sort();
        for job in dirs {
            let meta = read_meta(&job);
            let mut projects = Vec::new();
            let mut files: Vec<PathBuf> = std::fs::read_dir(&job).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("tlp"))).collect()).unwrap_or_default();
            files.sort();
            for pf in files {
                let stem = pf.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let gc = Self::gcode_for(&job, &stem);
                let exported = gc.iter().filter_map(|g| std::fs::metadata(g).and_then(|m| m.modified()).ok()).max();
                let modified = std::fs::metadata(&pf).and_then(|m| m.modified()).ok();
                let rev = self.revisions(&pf).iter().map(|r| r.rev).max().unwrap_or(0);
                projects.push(ProjectInfo {
                    name: stem,
                    path: pf,
                    modified: modified.map(fmt_time).unwrap_or_default(),
                    exported: exported.map(fmt_time).unwrap_or_default(),
                    files: gc.len(),
                    revision: if gc.is_empty() { 0 } else { rev + 1 },
                    stale: matches!((exported, modified), (Some(e), Some(m)) if m > e + std::time::Duration::from_secs(2)),
                });
            }
            self.jobs.push(JobInfo { name: job.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), path: job, status: meta.get("status").and_then(|v| v.as_str()).unwrap_or("Planned").to_string(), notes: meta.get("notes").and_then(|v| v.as_str()).unwrap_or("").to_string(), projects });
        }
    }

    pub fn create(&mut self, name: &str) -> Option<PathBuf> {
        let clean: String = name.trim().chars().map(|c| if matches!(c, '/' | '\\' | ':') { '-' } else { c }).collect();
        if clean.is_empty() {
            self.status = "Give the job a name".into();
            return None;
        }
        let job = self.root.join(&clean);
        if job.exists() {
            self.status = format!("A job called \"{clean}\" already exists");
            return None;
        }
        if std::fs::create_dir_all(job.join("gcode")).is_err() {
            self.status = format!("Cannot create {}", job.display());
            return None;
        }
        write_meta(&job, &serde_json::json!({"status": "Planned", "created": chrono::Local::now().to_rfc3339()}));
        self.status = format!("Created job {clean}");
        self.refresh();
        Some(job)
    }

    pub fn set_meta(&mut self, job: &Path, key: &str, value: &str) {
        let mut m = read_meta(job);
        if m.get(key).and_then(|v| v.as_str()) == Some(value) {
            return;
        }
        m[key] = serde_json::Value::String(value.to_string());
        write_meta(job, &m);
        self.refresh();
    }

    pub fn add_project(&mut self, job: &Path, src: &Path) -> Option<PathBuf> {
        let dst = job.join(src.file_name()?);
        if dst.exists() {
            self.status = format!("{} is already in this job", dst.file_name().unwrap_or_default().to_string_lossy());
            return None;
        }
        if std::fs::copy(src, &dst).is_err() {
            self.status = format!("Cannot copy {}", src.display());
            return None;
        }
        self.status = format!("Added {}", dst.file_name().unwrap_or_default().to_string_lossy());
        self.refresh();
        Some(dst)
    }

    /// Export the *saved* project file's G-code into `<job>/gcode`, archiving earlier revisions.
    pub fn export_project(&mut self, project: &Path, user_posts: &crate::posts::UserPosts) -> String {
        let Some(job) = self.job_for(project) else { return "Not inside a job folder".into() };
        let r = (|| -> Result<String, String> {
            let text = std::fs::read_to_string(project).map_err(|e| e.to_string())?;
            let doc = project::load_project(&text)?;
            let stem = project.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            export_to_dir(&doc, &stem, &job.join("gcode"), user_posts)
        })();
        self.status = r.unwrap_or_else(|e| e);
        self.refresh();
        self.status.clone()
    }

    pub fn export_job(&mut self, job: &Path, user_posts: &crate::posts::UserPosts) -> String {
        let projects: Vec<PathBuf> = self.jobs.iter().find(|j| j.path == job).map(|j| j.projects.iter().map(|p| p.path.clone()).collect()).unwrap_or_default();
        if projects.is_empty() {
            self.status = "No projects in this job yet".into();
            return self.status.clone();
        }
        let notes: Vec<String> = projects.iter().map(|p| format!("{}: {}", p.file_stem().unwrap_or_default().to_string_lossy(), self.export_project(p, user_posts))).collect();
        self.status = notes.join("\n");
        self.status.clone()
    }

    pub fn reveal(path: &Path) {
        crate::files::open_with_os(if path.is_dir() { path } else { path.parent().unwrap_or(path) });
    }
}

/// Regenerate + post the document into `dir` (files prefixed with `stem`), every existing file archived first.
/// Shared by the Jobs dialog and the app's export when the open project lives inside a job.
pub fn export_to_dir(doc: &gcarve_core::document::Document, stem: &str, dir: &Path, user_posts: &crate::posts::UserPosts) -> Result<String, String> {
    let gen = pipeline::regenerate(doc);
    let ts = chrono::Local::now().format("%I:%M %p, %A %B %-d %Y").to_string();
    let post = pipeline::active_post(doc, &|id| user_posts.find(id));
    let r = pipeline::export(doc, &gen, stem, &ts, 0.0, &post);
    if r.files.is_empty() {
        return Err("nothing to export".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut names = Vec::new();
    for f in &r.files {
        let p = dir.join(&f.filename);
        let rev = archive_revision(&p, &f.text);
        std::fs::write(&p, &f.text).map_err(|e| format!("Cannot write {}: {e}", p.display()))?;
        names.push(format!("{} (r{rev})", f.filename));
    }
    let mut msg = format!("Exported {}", names.join(", "));
    if !r.safety.is_empty() {
        msg += &format!(" ⚠ {}", r.safety.join("; "));
    }
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revisions_archive_only_real_changes() {
        let dir = std::env::temp_dir().join(format!("gcarve-jobs-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("sign_01_endmill.tap");
        assert_eq!(archive_revision(&f, "G0 Z1\n"), 1); // nothing there yet
        std::fs::write(&f, "[ 09:09 PM, Tuesday December 17 2024 ]\nG0 Z1\n").unwrap();
        assert_eq!(archive_revision(&f, "[ 10:00 AM, Monday January 6 2025 ]\nG0 Z1\n"), 1); // only the stamp differs
        assert!(f.exists());
        assert_eq!(archive_revision(&f, "G0 Z2\n"), 2);
        assert!(!f.exists());
        let hist: Vec<String> = std::fs::read_dir(dir.join("history")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(hist.len(), 1);
        assert!(hist[0].starts_with("sign_01_endmill.r1.") && hist[0].ends_with(".tap"), "{hist:?}");
        assert_eq!(revision_of(&hist[0]), 1);
        std::fs::write(&f, "G0 Z2\n").unwrap();
        assert_eq!(archive_revision(&f, "G0 Z3\n"), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn job_for_and_create() {
        let root = std::env::temp_dir().join(format!("gcarve-jobs-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut j = Jobs { root: root.clone(), jobs: Vec::new(), status: String::new() };
        let job = j.create("Smith / sign").unwrap();
        assert!(job.ends_with("Smith - sign") && job.join("gcode").is_dir());
        assert!(j.create("Smith / sign").is_none());
        assert_eq!(j.job_for(&job.join("a.tlp")), Some(job.clone()));
        assert_eq!(j.job_for(&root.join("a.tlp")), None);
        j.set_meta(&job, "status", "Cut");
        assert_eq!(j.jobs[0].status, "Cut");
        let _ = std::fs::remove_dir_all(&root);
    }
}
