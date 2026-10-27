//! First-time guide (owner 2026-10-26: most visitors stop at the demo): a five-step checklist at the top of the side
//! panel that ticks itself as the job moves along: a drawing, the machine, a toolpath, a look at the cut, the export.
//! Each open step has the button that does it. Hidden for good by "Hide the guide"; Help › Getting Started brings it back.

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use gcarve_core::document::Document;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tour {
    pub dismissed: bool,
    /// the Simulate view has been opened / G-code has been exported (these two leave nothing in the document)
    pub simulated: bool,
    pub exported: bool,
}

/// (what to do, why / how, the command the button runs, the button's label, done)
pub type Step = (&'static str, &'static str, &'static str, &'static str, bool);

impl Tour {
    const KEY: &'static str = "tour.json";

    /// What the first run of this version does: someone who already has settings saved is not new, so they start with
    /// the guide hidden (Help › Getting Started shows it); everybody else sees it.
    pub fn fresh(used_before: bool) -> Tour {
        Tour { dismissed: used_before, ..Default::default() }
    }

    pub fn load() -> Tour {
        if std::env::var_os("GCARVE_TOUR").is_some() {
            return Tour::fresh(false); // dev: look at the guide as a first visitor sees it
        }
        match crate::store::load(Self::KEY).and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
            Some(v) => {
                let b = |k: &str| v.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
                Tour { dismissed: b("dismissed"), simulated: b("simulated"), exported: b("exported") }
            }
            None => Tour::fresh(crate::store::load("ui.json").is_some()),
        }
    }

    pub fn save(&self) {
        crate::store::save(Self::KEY, &serde_json::json!({"dismissed": self.dismissed, "simulated": self.simulated, "exported": self.exported}).to_string());
    }

    pub fn steps(&self, doc: &Document) -> [Step; 5] {
        [
            (i18n::t("Start with a drawing"), i18n::t("Draw with the tools on the left, import a file, or open the demo to see a finished job."), "demo", i18n::t("Open the demo"), !doc.shapes.is_empty() || !doc.reliefs.is_empty()),
            (i18n::t("Pick your machine"), i18n::t("Feeds, board size and the G-code style follow it."), "machine", i18n::t("Choose machine…"), doc.material.machine.is_some()),
            (i18n::t("Add a toolpath"), i18n::t("Select shapes, then choose what to cut: profile, pocket, V-carve…"), "tab:toolpaths", i18n::t("Add toolpath"), !doc.ops.is_empty()),
            (i18n::t("Watch it cut"), i18n::t("The simulation shows the finished board before any wood is cut."), "mode:Simulate", i18n::t("Simulate"), self.simulated),
            (i18n::t("Export the G-code"), i18n::t("Save the file for your machine. Safety problems are listed first."), "export", i18n::t("Export…"), self.exported),
        ]
    }

    /// shown while it is not hidden and something is still to do
    pub fn active(&self, doc: &Document) -> bool {
        !self.dismissed && self.steps(doc).iter().any(|s| !s.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gcarve_core::document::{Op, OpType};
    use gcarve_core::geom::make_rect;

    #[test]
    fn the_guide_ticks_itself_off_and_goes_when_done_or_hidden() {
        let mut doc = Document::default();
        let mut t = Tour::fresh(false);
        let done = |t: &Tour, d: &Document| t.steps(d).iter().filter(|s| s.4).count();
        assert!(t.active(&doc) && done(&t, &doc) == 0);
        doc.add(make_rect(0.0, 0.0, 2.0, 1.0), None);
        assert_eq!(done(&t, &doc), 1);
        doc.ops.push(Op { kind: OpType::Profile, ..Default::default() });
        t.simulated = true;
        t.exported = true;
        assert_eq!(done(&t, &doc), 4); // no machine chosen yet
        assert!(t.active(&doc));
        doc.material.machine = Some(Default::default());
        assert!(!t.active(&doc), "everything done: the guide leaves by itself");
        let hidden = Tour::fresh(true);
        assert!(!hidden.active(&Document::default()), "someone who used the app before starts with it hidden");
    }
}
