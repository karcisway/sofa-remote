//! Everything this PC remembers lives in one JSON file, `.config-sofa-remote`, next to the
//! executable (or in --data DIR), so it can be read and edited by hand:
//!
//! ```json
//! { "token": "…", "admin": false, "layouts_version": 1, "default_tabs": ["Remote", …], "layouts": […] }
//! ```
//!
//! `layouts` is missing until a phone saves its own; until then the built-in default is used.
//! `default_tabs` lists the built-in tabs this file has already seen, so a newer version's new
//! tabs are added to saved layouts while tabs deleted on purpose stay deleted.
//! Older versions kept layouts.json, .remote_token and .remote_admin; they are moved in here.

use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const FILE: &str = ".config-sofa-remote";
const DEFAULT_LAYOUTS: &str = include_str!("../default_layouts.json");

/// Read-modify-write of the file happens under this lock.
static LOCK: Mutex<()> = Mutex::new(());

pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE)
}

fn read(data_dir: &Path) -> Map<String, Value> {
    let file = path(data_dir);
    match std::fs::read_to_string(&file) {
        Ok(text) => match serde_json::from_str(&text) {
            Ok(Value::Object(m)) => m,
            _ => {
                // Keep a broken hand edit instead of overwriting it
                let bad = data_dir.join(format!("{FILE}.broken"));
                eprintln!("{} isn't a valid JSON object; moved to {} and starting fresh", file.display(), bad.display());
                let _ = std::fs::rename(&file, bad);
                Map::new()
            }
        },
        Err(_) => migrate(data_dir),
    }
}

/// Settings from versions that used separate files. The old files are removed once saved here.
fn migrate(data_dir: &Path) -> Map<String, Value> {
    let mut m = Map::new();
    let token = data_dir.join(".remote_token");
    let admin = data_dir.join(".remote_admin");
    let layouts = data_dir.join("layouts.json");
    if let Ok(t) = std::fs::read_to_string(&token) {
        m.insert("token".into(), t.trim().into());
    }
    if admin.exists() {
        m.insert("admin".into(), true.into());
    }
    if let Some(Value::Object(old)) = std::fs::read_to_string(&layouts).ok().and_then(|t| serde_json::from_str(&t).ok()) {
        if let Some(v) = old.get("version") {
            m.insert("layouts_version".into(), v.clone());
        }
        if let Some(l) = old.get("layouts") {
            m.insert("layouts".into(), l.clone());
        }
        // No record of which built-in tabs it saw: default_tabs stays empty, so missing ones get added
    }
    if !m.is_empty() && write(data_dir, &m).is_ok() {
        println!("Moved the old settings files into {}", path(data_dir).display());
        for f in [token, admin, layouts] {
            let _ = std::fs::remove_file(f);
        }
    }
    m
}

fn write(data_dir: &Path, m: &Map<String, Value>) -> anyhow::Result<()> {
    let tmp = data_dir.join(format!("{FILE}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(m)?)?;
    std::fs::rename(tmp, path(data_dir))?;
    Ok(())
}

fn update<T>(data_dir: &Path, f: impl FnOnce(&mut Map<String, Value>) -> T) -> anyhow::Result<T> {
    let _g = LOCK.lock().unwrap();
    let mut m = read(data_dir);
    let out = f(&mut m);
    write(data_dir, &m)?;
    Ok(out)
}

/// The access key, made on first start. `renew` replaces it (--new-key).
pub fn token(data_dir: &Path, renew: bool, make: impl FnOnce() -> String) -> anyhow::Result<String> {
    update(data_dir, |m| match m.get("token").and_then(Value::as_str) {
        Some(t) if !renew && !t.trim().is_empty() => t.trim().to_string(),
        _ => {
            let t = make();
            m.insert("token".into(), t.clone().into());
            t
        }
    })
}

pub fn admin(data_dir: &Path) -> bool {
    let _g = LOCK.lock().unwrap();
    read(data_dir).get("admin").and_then(Value::as_bool).unwrap_or(false)
}

pub fn set_admin(data_dir: &Path, on: bool) {
    let _ = update(data_dir, |m| m.insert("admin".into(), on.into()));
}

fn defaults() -> Map<String, Value> {
    match serde_json::from_str(DEFAULT_LAYOUTS) {
        Ok(Value::Object(m)) => m,
        _ => unreachable!("default_layouts.json is a JSON object"),
    }
}

fn tab_name(tab: &Value) -> Option<&str> {
    tab.get("name").and_then(Value::as_str)
}

/// Does this file have its own layouts (rather than the built-in default)?
pub fn has_layouts(data_dir: &Path) -> bool {
    let _g = LOCK.lock().unwrap();
    read(data_dir).get("layouts").is_some_and(Value::is_array)
}

/// The layouts config the phone edits: `{ "version": n, "layouts": [...] }`. Built-in tabs this
/// file hasn't seen yet are appended to saved layouts (and remembered as seen).
pub fn layouts(data_dir: &Path) -> anyhow::Result<Value> {
    let def = defaults();
    let def_tabs: Vec<Value> = def.get("layouts").and_then(Value::as_array).cloned().unwrap_or_default();
    update(data_dir, |m| {
        let Some(Value::Array(tabs)) = m.get("layouts").cloned() else {
            return Value::Object(def.clone());
        };
        let seen: Vec<String> = m.get("default_tabs").and_then(Value::as_array).into_iter().flatten()
            .filter_map(|v| v.as_str().map(String::from)).collect();
        let mut tabs = tabs;
        for tab in &def_tabs {
            let Some(name) = tab_name(tab) else { continue };
            if !seen.iter().any(|s| s == name) && !tabs.iter().any(|t| tab_name(t) == Some(name)) {
                println!("Added the new built-in tab '{name}' to the saved layouts");
                tabs.push(tab.clone());
            }
        }
        let names: Vec<Value> = def_tabs.iter().filter_map(tab_name).map(Value::from).collect();
        m.insert("default_tabs".into(), names.into());
        m.insert("layouts".into(), tabs.clone().into());
        let version = m.get("layouts_version").cloned().or_else(|| def.get("version").cloned()).unwrap_or(json!(1));
        json!({ "version": version, "layouts": tabs })
    })
}

pub fn set_layouts(data_dir: &Path, config: &Value) -> anyhow::Result<()> {
    let tabs = config.get("layouts").filter(|l| l.is_array())
        .ok_or_else(|| anyhow::anyhow!("config must be an object with a 'layouts' list"))?.clone();
    let names: Vec<Value> = defaults().get("layouts").and_then(Value::as_array).into_iter().flatten()
        .filter_map(tab_name).map(Value::from).collect();
    update(data_dir, |m| {
        if let Some(v) = config.get("version") {
            m.insert("layouts_version".into(), v.clone());
        }
        m.insert("default_tabs".into(), names.into());
        m.insert("layouts".into(), tabs);
    })
}

/// Back to the built-in layouts; the key and other settings stay.
pub fn reset_layouts(data_dir: &Path) -> anyhow::Result<Value> {
    update(data_dir, |m| {
        for k in ["layouts", "layouts_version", "default_tabs"] {
            m.remove(k);
        }
    })?;
    Ok(Value::Object(defaults()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sofa-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn migrates_old_files_and_adds_new_default_tabs() {
        let d = dir("migrate");
        std::fs::write(d.join(".remote_token"), "abc\n").unwrap();
        std::fs::write(d.join(".remote_admin"), "1").unwrap();
        std::fs::write(d.join("layouts.json"), r#"{"version":1,"layouts":[{"name":"Mine"}]}"#).unwrap();
        assert_eq!(token(&d, false, || "new".into()).unwrap(), "abc");
        assert!(admin(&d));
        assert!(!d.join(".remote_token").exists() && !d.join("layouts.json").exists());
        let tabs = layouts(&d).unwrap()["layouts"].as_array().unwrap().len();
        assert_eq!(tabs, 1 + defaults()["layouts"].as_array().unwrap().len());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn deleted_default_tab_stays_deleted() {
        let d = dir("deleted");
        set_layouts(&d, &json!({ "version": 1, "layouts": [{ "name": "Mine" }] })).unwrap();
        assert_eq!(layouts(&d).unwrap()["layouts"].as_array().unwrap().len(), 1);
        assert!(!has_layouts(&dir("empty")));
        assert_eq!(reset_layouts(&d).unwrap(), Value::Object(defaults()));
        assert!(!has_layouts(&d));
        let _ = std::fs::remove_dir_all(&d);
    }
}
