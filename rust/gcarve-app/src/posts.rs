//! User post-processors (uploaded Vectric .pp or GCarve .json), stored next to the bit library; the built-ins come
//! from gcarve_core::postdef::post_library().

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
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
/// Post search: every word typed must be somewhere in the post's name, controller, machines, id, extension or units
/// (in any order: "shark mm", "fanuc atc", ".sbp"); punctuation is ignored, empty = all.
/// How well a post fits the search, for the order of the list (higher first); None = it doesn't. A word found in
/// the name counts more than one found in the machines list, a name that starts with the first word most.
pub fn score(p: &PostDef, q: &str) -> Option<u32> {
    let norm = |t: &str| t.to_lowercase().chars().map(|c| if c.is_alphanumeric() || c == '.' { c } else { ' ' }).collect::<String>();
    let units = if p.inches { "inch inches in imperial" } else { "mm metric millimetre millimeter" };
    let name = norm(&p.name);
    let rest = norm(&format!("{} {} {} .{} {} {}", p.controller, p.machines, p.id, p.extension, units, i18n::tr(&p.name)));
    let words: Vec<String> = norm(q).split_whitespace().map(str::to_string).collect();
    let mut score = 0;
    for (k, w) in words.iter().enumerate() {
        let w = w.trim_start_matches('.');
        let hit_name = name.split_whitespace().any(|t| t.trim_start_matches('.').starts_with(w)) || name.contains(w);
        if hit_name {
            score += if k == 0 && name.starts_with(w) { 4 } else { 2 };
        } else if rest.contains(w) {
            score += 1;
        } else {
            return None;
        }
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::score;
    use gcarve_core::postdef::{find_post, post_library};

    #[test]
    fn post_search_takes_words_in_any_order_and_puts_the_best_first() {
        let hits = |q: &str| post_library().iter().filter(|p| score(p, q).is_some()).map(|p| p.id.clone()).collect::<Vec<_>>();
        assert_eq!(hits("").len(), post_library().len());
        // two words, either order, one of them only in the machines list
        assert!(hits("shark inch").contains(&"nextwave-in".to_string()) && hits("inch shark").contains(&"nextwave-in".to_string()));
        assert!(hits(".sbp").iter().all(|i| i.starts_with("shopbot")) && !hits(".sbp").is_empty());
        assert!(hits("komo mm") == vec!["komo-mm".to_string()], "{:?}", hits("komo mm"));
        assert!(hits("winpc").contains(&"winpc-mm".to_string()) && hits("stepcraft").contains(&"winpc-mm".to_string()));
        assert!(hits("zzzz nothing").is_empty());
        // a post named after the word ranks above one that only lists it among its machines
        let named = score(find_post("laguna-fanuc-in").unwrap(), "laguna").unwrap();
        let listed = score(find_post("wincnc-atc").unwrap(), "laguna").unwrap();
        assert!(named > listed);
    }
}
