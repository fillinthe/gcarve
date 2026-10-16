//! The user's own bits (toollib.rs) and custom machines, kept locally and with the account
//! (/api/account/library) so every device has them, and sharing them by email (/api/shares).

use gcarve_core::document::{MachineLimits, Tool};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

const MACHINES_KEY: &str = "machines.json";

/// bumped on every local library save: the app uploads when it moves (App::sync_library)
static REV: AtomicU64 = AtomicU64::new(1);

pub fn rev() -> u64 {
    REV.load(Ordering::Relaxed)
}

pub fn touched() {
    REV.fetch_add(1, Ordering::Relaxed);
}

/// A machine the user set up (not in the database, or a database machine with their own numbers)
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomMachine {
    pub limits: MachineLimits,
    /// post-processor id ("" = keep the project's)
    pub post: String,
}

pub fn machines() -> Vec<CustomMachine> {
    crate::store::load(MACHINES_KEY).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save_machines(m: &[CustomMachine]) {
    crate::store::save(MACHINES_KEY, &serde_json::to_string(m).unwrap_or_default());
    touched();
}

/// Add or replace by name; returns true when it was new
pub fn upsert_machines(list: &mut Vec<CustomMachine>, incoming: Vec<CustomMachine>) -> usize {
    let mut added = 0;
    for m in incoming {
        match list.iter_mut().find(|x| x.limits.name.eq_ignore_ascii_case(&m.limits.name)) {
            Some(x) => *x = m,
            None => {
                list.push(m);
                added += 1;
            }
        }
    }
    added
}

pub fn bits_json(tools: &[Tool]) -> serde_json::Value {
    serde_json::from_str(&gcarve_core::project::tool_library_to_json(tools)).unwrap_or_default()
}

pub fn bits_from(v: &serde_json::Value) -> Vec<Tool> {
    gcarve_core::project::tool_library_from_json(&v.to_string())
}

/// The account copy arrived (None = never saved there): merge it into the local one; the merged result goes back up.
pub fn merge_from_account(bits: Option<&serde_json::Value>, machines: Option<&serde_json::Value>) {
    if let Some(v) = bits {
        let mut local = crate::toollib::load();
        crate::toollib::merge(&mut local, bits_from(v));
        crate::toollib::save(&local);
    }
    if let Some(v) = machines {
        let mut local = self::machines();
        upsert_machines(&mut local, serde_json::from_value(v.clone()).unwrap_or_default());
        save_machines(&local);
    }
    touched();
}

/// A share someone accepted: add it to the library. Returns the status-bar message.
pub fn accept(kind: &str, name: &str, data: &serde_json::Value) -> String {
    match kind {
        "bits" => {
            let mut local = crate::toollib::load();
            let (a, u) = crate::toollib::merge(&mut local, bits_from(data));
            crate::toollib::save(&local);
            format!("“{name}”: {a} bit(s) added, {u} updated in your library")
        }
        "machines" => {
            let mut local = machines();
            let n = upsert_machines(&mut local, serde_json::from_value(data.clone()).unwrap_or_default());
            save_machines(&local);
            format!("“{name}”: {n} machine(s) added to My machines")
        }
        _ => "Unknown share".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machines_replace_by_name() {
        let m = |n: &str, rpm: f64| CustomMachine { limits: MachineLimits { name: n.into(), rpm_max: rpm, ..Default::default() }, post: String::new() };
        let mut list = vec![m("Shop", 18000.0)];
        assert_eq!(upsert_machines(&mut list, vec![m("shop", 24000.0), m("Garage", 10000.0)]), 1);
        assert_eq!((list.len(), list[0].limits.rpm_max), (2, 24000.0));
    }

    #[test]
    fn bits_round_trip_as_json() {
        let t = vec![Tool { name: "1/4 down".into(), diameter: 0.25, ..Default::default() }];
        assert_eq!(bits_from(&bits_json(&t))[0].name, "1/4 down");
    }
}
