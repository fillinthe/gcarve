//! Toolpath template library (Qt Canvas::templateList & co). Desktop: `.gctemplate` files in <config>/templates (env
//! GCARVE_TEMPLATES_DIR overrides, as in the Qt app). Web: the account (/api/templates) when signed in, else browser
//! storage. The templates themselves (save / apply / JSON) are gcarve_core::template.

use gcarve_core::template::{load_template, save_template, ToolpathTemplate};

pub struct Entry {
    /// file path (desktop), server id (account) or "" (browser storage)
    pub key: String,
    pub t: ToolpathTemplate,
}

pub use gcarve_ui::cloud::CloudOp;

#[derive(Default)]
pub struct Templates {
    pub items: Vec<Entry>,
    /// web + signed in: the library lives in the account
    pub account: bool,
    pub pending: Vec<CloudOp>,
}

#[cfg(target_arch = "wasm32")]
const LOCAL_KEY: &str = "templates.json";

impl Templates {
    pub fn reload(&mut self) {
        if self.account {
            self.pending.push(CloudOp::List);
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut items: Vec<Entry> = std::fs::read_dir(dir())
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "gctemplate"))
                .filter_map(|p| Some(Entry { t: load_template(&std::fs::read_to_string(&p).ok()?).ok()?, key: p.to_string_lossy().into() }))
                .collect();
            items.sort_by_key(|e| (e.t.name.to_lowercase(), e.key.clone()));
            self.items = items;
        }
        #[cfg(target_arch = "wasm32")]
        {
            let texts: Vec<String> = crate::store::load(LOCAL_KEY).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
            self.items = texts.iter().filter_map(|t| load_template(t).ok()).map(|t| Entry { key: String::new(), t }).collect();
        }
    }

    pub fn add(&mut self, t: ToolpathTemplate) -> Result<(), String> {
        if self.account {
            self.pending.push(CloudOp::Put { id: String::new(), name: t.name.clone(), json: save_template(&t) });
            return Ok(());
        }
        #[cfg(not(target_arch = "wasm32"))]
        std::fs::write(free_path(&t.name), save_template(&t)).map_err(|e| format!("Cannot save the template: {e}"))?;
        #[cfg(target_arch = "wasm32")]
        self.items.push(Entry { key: String::new(), t });
        self.persist();
        Ok(())
    }

    pub fn rename(&mut self, i: usize, name: &str) {
        let Some(e) = self.items.get_mut(i) else { return };
        e.t.name = name.trim().to_string();
        let json = save_template(&e.t);
        if self.account {
            self.pending.push(CloudOp::Put { id: e.key.clone(), name: e.t.name.clone(), json });
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if std::fs::write(&e.key, json).is_ok() && std::path::Path::new(&e.key).file_stem().is_none_or(|s| s.to_string_lossy() != e.t.name) {
            let _ = std::fs::rename(&e.key, free_path(&e.t.name));
        }
        self.persist();
    }

    pub fn remove(&mut self, i: usize) {
        if i >= self.items.len() {
            return;
        }
        let e = self.items.remove(i);
        if self.account {
            self.pending.push(CloudOp::Delete(e.key));
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = std::fs::remove_file(&e.key);
        self.persist();
    }

    /// Imported file: added to the library like a save.
    pub fn import(&mut self, text: &str) -> Result<String, String> {
        let t = load_template(text)?;
        let name = t.name.clone();
        self.add(t).map(|_| name)
    }

    /// account replies (Cloud::poll events)
    pub fn on_list(&mut self, ids: Vec<String>) {
        self.items.clear();
        self.pending.extend(ids.into_iter().map(CloudOp::Get));
    }
    pub fn on_data(&mut self, id: String, json: &str) {
        if let Ok(t) = load_template(json) {
            self.items.retain(|e| e.key != id);
            self.items.push(Entry { key: id, t });
            self.items.sort_by_key(|e| (e.t.name.to_lowercase(), e.key.clone()));
        }
    }

    /// desktop: the folder is the store, so re-read it; web without an account: browser storage
    fn persist(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        self.reload();
        #[cfg(target_arch = "wasm32")]
        crate::store::save(LOCAL_KEY, &serde_json::to_string(&self.items.iter().map(|e| save_template(&e.t)).collect::<Vec<_>>()).unwrap_or_default());
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn dir() -> std::path::PathBuf {
    let d = std::env::var("GCARVE_TEMPLATES_DIR").map(std::path::PathBuf::from).unwrap_or_else(|_| crate::store::config_dir().unwrap_or_default().join("templates"));
    let _ = std::fs::create_dir_all(&d);
    d
}

/// "<name>.gctemplate", then " 2", " 3"... when taken
#[cfg(not(target_arch = "wasm32"))]
fn free_path(name: &str) -> std::path::PathBuf {
    let base: String = name.chars().map(|c| if "\\/:*?\"<>|".contains(c) { '-' } else { c }).collect::<String>().trim().to_string();
    let base = if base.is_empty() { "Template".to_string() } else { base };
    let mut p = dir().join(format!("{base}.gctemplate"));
    let mut n = 2;
    while p.exists() {
        p = dir().join(format!("{base} {n}.gctemplate"));
        n += 1;
    }
    p
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use gcarve_core::template::make_template;

    #[test]
    fn folder_library() {
        let d = std::env::temp_dir().join(format!("gcarve-templates-{}", std::process::id()));
        std::env::set_var("GCARVE_TEMPLATES_DIR", &d);
        let doc = gcarve_core::project::load_project(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../samples/showcase.tlp")).unwrap()).unwrap();
        let mut lib = Templates::default();
        lib.add(make_template(&doc, "Sign", "")).unwrap();
        lib.add(make_template(&doc, "Sign", "again")).unwrap();
        assert_eq!(lib.items.len(), 2);
        let second = lib.items.iter().position(|e| e.key.ends_with("Sign 2.gctemplate")).expect("second file named Sign 2");
        lib.rename(second, "Panel");
        assert_eq!(lib.items.iter().map(|e| e.t.name.as_str()).collect::<Vec<_>>(), ["Panel", "Sign"]);
        assert!(lib.items[0].key.ends_with("Panel.gctemplate"));
        lib.remove(0);
        assert_eq!(lib.items.len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }
}
