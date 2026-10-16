//! User post-processors (uploaded Vectric .pp or GCarve .json), stored next to the bit library; the built-ins come
//! from gcarve_core::postdef::post_library().

use gcarve_core::postdef::{post_from_json, post_from_vectric_pp, post_library, post_to_json, PostDef};

const KEY: &str = "posts.json";

#[derive(Default)]
pub struct UserPosts {
    pub posts: Vec<PostDef>,
}

impl UserPosts {
    pub fn load() -> Self {
        let posts = crate::store::load(KEY).and_then(|t| serde_json::from_str::<Vec<PostDef>>(&t).ok()).unwrap_or_default();
        UserPosts { posts }
    }
    fn save(&self) {
        crate::store::save(KEY, &serde_json::to_string_pretty(&self.posts).unwrap_or_default());
    }
    pub fn find(&self, id: &str) -> Option<PostDef> {
        self.posts.iter().find(|p| p.id == id).cloned()
    }
    /// Upload: `.pp` is translated, `.json` parsed; the id gets a `user-` prefix (unique against the built-ins) and
    /// replaces an earlier upload with the same id. Returns the new id.
    pub fn add(&mut self, name: &str, text: &str) -> Result<String, String> {
        let mut p = if name.to_ascii_lowercase().ends_with(".pp") { post_from_vectric_pp(text)? } else { post_from_json(text)? };
        if p.id.is_empty() || p.id == "custom" || post_library().iter().any(|b| b.id == p.id) {
            p.id = name.rsplit_once('.').map(|(a, _)| a).unwrap_or(name).to_ascii_lowercase().replace(|c: char| !c.is_ascii_alphanumeric(), "-");
        }
        if !p.id.starts_with("user-") {
            p.id = format!("user-{}", p.id);
        }
        if p.name.is_empty() {
            p.name = p.id.clone();
        }
        let id = p.id.clone();
        self.posts.retain(|x| x.id != id);
        self.posts.push(p);
        self.save();
        Ok(id)
    }
    pub fn remove(&mut self, id: &str) {
        self.posts.retain(|p| p.id != id);
        self.save();
    }
    /// Built-ins then user posts (`custom` = true).
    pub fn all(&self) -> Vec<(PostDef, bool)> {
        post_library().iter().map(|p| (p.clone(), false)).chain(self.posts.iter().map(|p| (p.clone(), true))).collect()
    }
    pub fn export_json(&self, id: &str) -> Option<String> {
        post_library().iter().find(|p| p.id == id).cloned().or_else(|| self.find(id)).map(|p| post_to_json(&p))
    }
}

/// Search text like the Qt dialog: name, controller, machines, extension, units.
pub fn matches(p: &PostDef, q: &str) -> bool {
    let q = q.trim().to_ascii_lowercase();
    q.is_empty() || format!("{} {} {} {} {}", p.name, p.controller, p.machines, p.extension, if p.inches { "inches in" } else { "mm metric" }).to_ascii_lowercase().contains(&q)
}
