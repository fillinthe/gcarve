//! Account + cloud documents (gcarve server/ at <base>/api; one account for GCarve and GSketch), desktop and web from
//! one client (ehttp). Each app keeps its own document list (`Cloud::docs`: GCarve "projects", GSketch "sketches").
//! Web: same origin, the browser sends the HttpOnly session cookie. Desktop: bearer token from a "desktop" login,
//! kept in the app's storage dir.

use eframe::egui;
use serde::Deserialize;
use std::sync::mpsc::{channel, Receiver, Sender};

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Entitlements {
    #[serde(default)]
    pub plan: String,
    #[serde(default, rename = "maxProjects")]
    pub max_projects: i32,
    #[serde(default)]
    pub desktop: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct User {
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub role: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ProjectInfo {
    pub id: String,
    pub name: String,
    /// "Kitchen/Doors"; "" = top level
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub updated: i64,
}

#[derive(Deserialize, Default)]
pub struct AuthReply {
    #[serde(default)]
    user: Option<User>,
    #[serde(default)]
    entitlements: Option<Entitlements>,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    error: Option<String>,
    /// the account has two-step sign-in: ask for the code and send it again
    #[serde(default, rename = "twoFactor")]
    two_factor: bool,
    /// sign-up done, but the address must answer the emailed link first (no session yet)
    #[serde(default)]
    confirm: bool,
}

pub enum Reply {
    Auth(AuthReply),
    SignedOut,
    Expired,
    Projects(Vec<ProjectInfo>),
    Saved { id: String, name: String },
    /// a rename or move answered: reload the list
    Moved,
    Opened { id: String, name: String, json: String },
    /// account app defaults (GET /api/account/prefs): Some(object, empty when the account has none); None = a save
    /// answered or the fetch failed
    Prefs(Option<serde_json::Value>),
    /// toolpath template library in the account (templates.rs): the list, one template's JSON, a change to reload after
    Templates(Vec<ProjectInfo>),
    TemplateData { id: String, json: String },
    TemplatesChanged,
    /// Help ▸ Request a Change: Ok = stored (message id), Err = why not
    RequestSent(Result<i64, String>),
    /// the account's bits and machines (library.rs); None = never saved there
    Library { bits: Option<serde_json::Value>, machines: Option<serde_json::Value> },
    /// things shared with this account, waiting to be added or turned down
    Shares(Vec<ShareInfo>),
    /// a share was sent (message for the status bar)
    Shared(String),
    /// a share was taken: its data when accepted, None when declined
    ShareTaken { kind: String, name: String, data: Option<serde_json::Value> },
    /// the site's setting: sign-in required for the web app
    Config { require_signin: bool },
    /// cloud storage used / allowed (bytes) and project count / allowed
    Storage { used: u64, max: u64, projects: u32, max_projects: u32 },
    /// sign-in through the browser (server device.go): started (poll id, code, page), still waiting, over (the
    /// sign-in answer, or why not)
    DeviceStarted { id: String, code: String, url: String, secs: i64 },
    DevicePending,
    DeviceDone(AuthReply),
    DeviceFailed(String),
    /// a community project (gcarve.app/projects/<slug>/): its title and .tlp text
    CommunityOpened { slug: String, title: String, json: String },
    /// Share to Community answered: the project page's path (waiting for review), or why not
    CommunityShared(Result<String, String>),
    /// a drawing left for another app of the same account (POST /api/handover): its id, or why not
    HandoverSent(Result<String, String>),
    /// a drawing another app left (GET /api/handover/{id}): name, sending app's data (e.g. SVG text)
    HandoverData { name: String, data: String },
    Error(String),
}

/// "Sign in with your browser" in progress: the app shows `code`, the browser page shows it too
pub struct Device {
    id: String,
    pub code: String,
    pub url: String,
    until: i64,
    /// unix seconds of the next poll; a poll in flight waits for its answer
    next: i64,
    waiting: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ShareInfo {
    pub id: i64,
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub name: String,
}

fn error_of(status: u16, text: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
    v.get("error").and_then(|e| e.as_str()).map_or_else(|| format!("Request failed ({status})"), String::from)
}

/// GET /api/<list> answer: the entries, or the server's error
fn list_reply(status: u16, text: &str, key: &str) -> Result<Vec<ProjectInfo>, String> {
    let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
    match v.get(key).cloned().map(serde_json::from_value::<Vec<ProjectInfo>>) {
        Some(Ok(l)) if status < 400 => Ok(l),
        _ => Err(v.get("error").and_then(|e| e.as_str()).map_or_else(|| format!("Request failed ({status})"), String::from)),
    }
}

#[derive(Deserialize, Default)]
struct Doc {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    data: String,
    #[serde(default)]
    error: Option<String>,
}
/// one project / template (GET, POST, PUT answers)
fn doc_reply(status: u16, text: &str) -> Result<Doc, String> {
    let d: Doc = serde_json::from_str(text).unwrap_or_default();
    if status < 400 { Ok(d) } else { Err(d.error.unwrap_or_else(|| format!("Request failed ({status})"))) }
}

/// server/api.go projectDocs.maxBytes
const MAX_PROJECT_BYTES: usize = 20 << 20;

pub struct Cloud {
    pub base: String,
    pub user: Option<User>,
    pub ent: Entitlements,
    pub projects: Vec<ProjectInfo>,
    pub error: String,
    pub busy: u32,
    /// bumped whenever who is signed in changes: a 401 to a request sent before that is stale, not "your sign-in expired"
    auth_gen: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// the last sign-in needs the authenticator code (two-step sign-in)
    pub need_code: bool,
    pub checked: bool,
    /// the site asks for an account before the web app can be used (admin setting; read at start)
    pub require_signin: bool,
    /// the sign-in ended while working (idle timeout): the app asks to sign in again, once
    pub expired: bool,
    /// (bytes used, bytes allowed, projects, projects allowed) from the last refresh
    pub storage: Option<(u64, u64, u32, u32)>,
    /// this app's document list on the server: "projects" (GCarve) or "sketches" (GSketch)
    pub docs: &'static str,
    /// a browser sign-in is waiting for the person to allow it (desktop)
    pub device: Option<Device>,
    token: String,
    tx: Sender<Reply>,
    rx: Receiver<Reply>,
}

/// Requests for the account library; the app hands them to `Cloud::template_op` (replies come back via `on_*`).
pub enum CloudOp {
    List,
    Get(String),
    Put { id: String, name: String, json: String },
    Delete(String),
}

impl Default for Cloud {
    fn default() -> Self {
        Self::new()
    }
}

impl Cloud {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        #[cfg(target_arch = "wasm32")]
        let base = web_sys::window().and_then(|w| w.location().origin().ok()).unwrap_or_default();
        #[cfg(not(target_arch = "wasm32"))]
        let base = std::env::var("GCARVE_CLOUD_URL").unwrap_or_else(|_| "https://gcarve.app".into());
        let mut c = Cloud { base, user: None, ent: Entitlements::default(), projects: Vec::new(), error: String::new(), busy: 0, auth_gen: Default::default(), need_code: false, checked: false, require_signin: false, expired: false, storage: None, docs: "projects", device: None, token: String::new(), tx, rx };
        c.token = c.load_token();
        c
    }

    pub fn web() -> bool {
        cfg!(target_arch = "wasm32")
    }
    /// the web build served from this computer (local testing): the sign-in box can be closed there
    pub fn local(&self) -> bool {
        Self::web() && (self.base.contains("://127.0.0.1") || self.base.contains("://localhost"))
    }
    pub fn signed_in(&self) -> bool {
        self.user.is_some()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn token_path() -> Option<std::path::PathBuf> {
        crate::store::config_dir().map(|d| d.join("account.token"))
    }

    /// Desktop licence (Qt CloudClient::licensed): GCarve Pro or an admin account confirmed online now, or within the
    /// last GRACE_DAYS with a stored token (shop PCs keep working offline). The web only needs a session.
    pub fn licensed(&self) -> bool {
        Self::web() && self.signed_in() || licensed_at(self.signed_in() && self.desktop_allowed(), !self.token.is_empty(), verified_at(), now())
    }
    pub fn grace_days_left(&self) -> i64 {
        grace_days_left(verified_at(), now())
    }
    /// the plan includes the desktop app (server entitlement), or the account is an admin
    pub fn desktop_allowed(&self) -> bool {
        self.ent.desktop || self.user.as_ref().is_some_and(|u| u.role == "admin" || u.role == "superadmin")
    }
    fn load_token(&self) -> String {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(p) = Self::token_path() {
            return std::fs::read_to_string(p).unwrap_or_default().trim().to_string();
        }
        String::new()
    }
    fn store_token(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(p) = Self::token_path() {
            if self.token.is_empty() {
                let _ = std::fs::remove_file(&p);
            } else if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
                let _ = std::fs::write(&p, &self.token);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)); // a sign-in, not for other users
                }
            }
        }
    }

    fn request(&mut self, ctx: &egui::Context, method: &str, path: &str, body: Option<serde_json::Value>, on: impl 'static + Send + FnOnce(u16, &str) -> Reply) {
        let mut req = match body {
            Some(b) => ehttp::Request::post(format!("{}{}", self.base, path), serde_json::to_vec(&b).unwrap_or_default()),
            None => ehttp::Request::get(format!("{}{}", self.base, path)),
        };
        req.method = method.to_string();
        req.headers.insert("Content-Type", "application/json");
        if !self.token.is_empty() {
            req.headers.insert("Authorization", format!("Bearer {}", self.token));
            req.headers.insert("X-App", crate::store::app()); // desktop apps share one server address: say which app this is
        }
        #[cfg(target_arch = "wasm32")]
        {
            req.mode = ehttp::Mode::SameOrigin; // the session cookie travels with same-origin requests
        }
        self.busy += 1;
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let auth = path.starts_with("/api/auth/");
        let (gen, sent_at) = (self.auth_gen.clone(), self.auth_gen.load(std::sync::atomic::Ordering::Relaxed));
        ehttp::fetch(req, move |r| {
            let reply = match r {
                // the sign-in timed out (idle) or was ended elsewhere; unless someone signed in since this was sent
                Ok(resp) if resp.status == 401 && !auth && gen.load(std::sync::atomic::Ordering::Relaxed) == sent_at => Reply::Expired,
                Ok(resp) if resp.status == 401 && !auth => Reply::Error(String::new()),
                Ok(resp) => on(resp.status, resp.text().unwrap_or("")),
                Err(e) => Reply::Error(format!("Can't reach {} right now ({e}). Check the connection; your drawing is safe on this computer.", crate::store::app())),
            };
            let _ = tx.send(reply);
            ctx.request_repaint();
        });
    }

    fn auth_reply(status: u16, text: &str) -> Reply {
        let mut a: AuthReply = serde_json::from_str(text).unwrap_or_default();
        if status >= 400 && a.error.is_none() {
            a.error = Some(format!("Request failed ({status})"));
        }
        Reply::Auth(a)
    }

    /// Public site settings (sign-in required on the web?); the answer arrives as `Reply::Config`.
    pub fn fetch_config(&mut self, ctx: &egui::Context) {
        if !Self::web() {
            return;
        }
        self.request(ctx, "GET", "/api/config", None, |status, text| {
            let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
            Reply::Config { require_signin: status < 400 && v.get("requireSignin").and_then(serde_json::Value::as_bool).unwrap_or(false) }
        });
    }

    pub fn check_session(&mut self, ctx: &egui::Context) {
        if !Self::web() && self.token.is_empty() {
            self.checked = true;
            return;
        }
        self.request(ctx, "GET", "/api/auth/me", None, Self::auth_reply);
    }

    pub fn sign_in(&mut self, ctx: &egui::Context, email: &str, password: &str, code: &str) {
        let body = serde_json::json!({"email": email.trim(), "password": password, "code": code.trim(), "client": if Self::web() { "web" } else { "desktop" }, "label": host_label()});
        self.request(ctx, "POST", "/api/auth/login", Some(body), Self::auth_reply);
    }

    pub fn register(&mut self, ctx: &egui::Context, email: &str, password: &str, invite: &str) {
        let body = serde_json::json!({"email": email.trim(), "password": password, "invite": invite.trim(), "client": if Self::web() { "web" } else { "desktop" }, "label": host_label()});
        self.request(ctx, "POST", "/api/auth/register", Some(body), Self::auth_reply);
    }

    /// Desktop: sign in with whatever the account uses (email link, passkey, password) on the website. Opens the
    /// browser at the server's confirm page; `poll` asks every 2 s until it is allowed, cancelled or 10 min pass.
    pub fn browser_sign_in(&mut self, ctx: &egui::Context) {
        self.error.clear();
        let body = serde_json::json!({"label": host_label(), "app": crate::store::app()});
        self.request(ctx, "POST", "/api/auth/device/start", Some(body), |status, text| {
            let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
            let s = |k: &str| v.get(k).and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
            if status == 200 && !s("id").is_empty() { Reply::DeviceStarted { id: s("id"), code: s("code"), url: s("url"), secs: v.get("expires").and_then(serde_json::Value::as_i64).unwrap_or(600) } } else { Reply::DeviceFailed(error_of(status, text)) }
        });
    }

    pub fn cancel_browser_sign_in(&mut self) {
        self.device = None;
    }

    fn poll_device(&mut self, ctx: &egui::Context) {
        let t = now();
        let Some(d) = self.device.as_mut() else { return };
        if t > d.until {
            self.device = None;
            self.error = "The browser sign-in took too long: press “Sign in with your browser” again.".into();
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_secs(2));
        if d.waiting || t < d.next {
            return;
        }
        d.waiting = true;
        d.next = t + 2;
        let body = serde_json::json!({"id": d.id});
        self.request(ctx, "POST", "/api/auth/device/poll", Some(body), |status, text| match status {
            202 => Reply::DevicePending,
            410 => Reply::DeviceFailed(error_of(status, text)),
            _ => match Self::auth_reply(status, text) {
                Reply::Auth(a) => Reply::DeviceDone(a),
                other => other,
            },
        });
    }

    pub fn sign_out(&mut self, ctx: &egui::Context) {
        self.request(ctx, "POST", "/api/auth/logout", Some(serde_json::json!({})), |_, _| Reply::SignedOut);
    }

    pub fn refresh(&mut self, ctx: &egui::Context) {
        let docs = self.docs;
        self.request(ctx, "GET", &format!("/api/{docs}"), None, move |status, text| list_reply(status, text, docs).map_or_else(Reply::Error, Reply::Projects));
        if self.signed_in() {
            self.request(ctx, "GET", "/api/account/storage", None, |status, text| {
                let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
                let n = |k: &str| v.get(k).and_then(serde_json::Value::as_u64).unwrap_or(0);
                if status < 400 { Reply::Storage { used: n("usedBytes"), max: n("maxBytes"), projects: n("projects") as u32, max_projects: n("maxProjects") as u32 } } else { Reply::Storage { used: 0, max: 0, projects: 0, max_projects: 0 } }
            });
        }
    }

    /// `folder`: Some for a new project (where it goes), None keeps the folder of an existing one
    pub fn save_project(&mut self, ctx: &egui::Context, id: &str, name: &str, folder: Option<&str>, json: &str) {
        // the server's limit, said before the upload instead of after a long one
        if json.len() > MAX_PROJECT_BYTES {
            self.error = format!("This project is {:.1} MB and cloud projects can be at most {} MB. Save it to this computer instead (File › Download Project), or remove big pictures or traced images from it.", json.len() as f64 / 1_048_576.0, MAX_PROJECT_BYTES >> 20);
            return;
        }
        let mut body = serde_json::json!({"name": name, "data": json});
        if let Some(f) = folder {
            body["folder"] = f.into();
        }
        let (method, path) = if id.is_empty() { ("POST", format!("/api/{}", self.docs)) } else { ("PUT", format!("/api/{}/{id}", self.docs)) };
        self.request(ctx, method, &path, Some(body), |status, text| doc_reply(status, text).map_or_else(Reply::Error, |p| Reply::Saved { id: p.id, name: p.name }));
    }

    /// Rename and/or move a project without touching its contents
    pub fn update_project(&mut self, ctx: &egui::Context, id: &str, name: Option<&str>, folder: Option<&str>) {
        let mut body = serde_json::json!({});
        if let Some(n) = name {
            body["name"] = n.into();
        }
        if let Some(f) = folder {
            body["folder"] = f.into();
        }
        self.request(ctx, "PUT", &format!("/api/{}/{id}", self.docs), Some(body), |status, text| if status < 400 { Reply::Moved } else { doc_reply(status, text).err().map_or(Reply::Moved, Reply::Error) });
    }

    pub fn open_project(&mut self, ctx: &egui::Context, id: &str) {
        self.request(ctx, "GET", &format!("/api/{}/{id}", self.docs), None, |status, text| doc_reply(status, text).map_or_else(Reply::Error, |p| Reply::Opened { id: p.id, name: p.name, json: p.data }));
    }

    pub fn delete_project(&mut self, ctx: &egui::Context, id: &str) {
        self.request(ctx, "DELETE", &format!("/api/{}/{id}", self.docs), None, |status, text| {
            if status < 400 { Reply::Projects(Vec::new()) } else { Reply::Error(format!("Delete failed ({status}): {text}")) }
        });
    }

    /// Toolpath templates in the account (/api/templates: same CRUD as projects, server/api.go docKind)
    pub fn template_op(&mut self, ctx: &egui::Context, op: CloudOp) {
        let changed = |status, text: &str| doc_reply(status, text).map_or_else(Reply::Error, |_| Reply::TemplatesChanged);
        match op {
            CloudOp::List => self.request(ctx, "GET", "/api/templates", None, |status, text| list_reply(status, text, "templates").map_or_else(Reply::Error, Reply::Templates)),
            CloudOp::Get(id) => self.request(ctx, "GET", &format!("/api/templates/{id}"), None, |status, text| doc_reply(status, text).map_or_else(Reply::Error, |p| Reply::TemplateData { id: p.id, json: p.data })),
            CloudOp::Put { id, name, json } => {
                let (method, path) = if id.is_empty() { ("POST", "/api/templates".to_string()) } else { ("PUT", format!("/api/templates/{id}")) };
                self.request(ctx, method, &path, Some(serde_json::json!({"name": name, "data": json})), changed);
            }
            CloudOp::Delete(id) => self.request(ctx, "DELETE", &format!("/api/templates/{id}"), None, |status, text| if status < 400 { Reply::TemplatesChanged } else { Reply::Error(format!("Delete failed ({status}): {text}")) }),
        }
    }

    /// Account app defaults (new-document board, grid, snaps; Qt CloudClient::fetchPrefs) -> Reply::Prefs
    pub fn fetch_prefs(&mut self, ctx: &egui::Context) {
        self.request(ctx, "GET", "/api/account/prefs", None, |status, text| {
            let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
            Reply::Prefs((status == 200).then(|| v.get("prefs").filter(|p| p.is_object()).cloned().unwrap_or_else(|| serde_json::json!({}))))
        });
    }
    /// Qt CloudClient::pushPrefs: PUT {"prefs": …} (signed in only)
    pub fn push_prefs(&mut self, ctx: &egui::Context, prefs: serde_json::Value) {
        if self.signed_in() {
            self.request(ctx, "PUT", "/api/account/prefs", Some(serde_json::json!({ "prefs": prefs })), |_, _| Reply::Prefs(None));
        }
    }

    /// GET /api/account/library -> Reply::Library
    pub fn fetch_library(&mut self, ctx: &egui::Context) {
        self.request(ctx, "GET", "/api/account/library", None, |status, text| {
            if status >= 400 {
                return Reply::Error(error_of(status, text));
            }
            let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
            let get = |k: &str| v.get(k).filter(|x| !x.is_null()).cloned();
            Reply::Library { bits: get("bits"), machines: get("machines") }
        });
    }

    /// PUT /api/account/library/{bits|machines}
    pub fn push_library(&mut self, ctx: &egui::Context, kind: &str, value: serde_json::Value) {
        if self.signed_in() {
            self.request(ctx, "PUT", &format!("/api/account/library/{kind}"), Some(value), |status, text| if status < 400 { Reply::Prefs(None) } else { Reply::Error(error_of(status, text)) });
        }
    }

    /// POST /api/shares: `kind` bits | machines, `data` what the recipient adds
    pub fn share(&mut self, ctx: &egui::Context, to: &str, kind: &str, name: &str, data: serde_json::Value) {
        let body = serde_json::json!({"to": to, "kind": kind, "name": name, "data": data});
        let msg = format!("Shared “{name}” with {to}: it waits in their {} until they add it", crate::store::app());
        self.request(ctx, "POST", "/api/shares", Some(body), move |status, text| if status < 400 { Reply::Shared(msg) } else { Reply::Error(error_of(status, text)) });
    }

    /// GET /api/community/<slug>/project: a shared project (public, no sign-in needed)
    pub fn community_open(&mut self, ctx: &egui::Context, slug: &str) {
        let slug_ = slug.to_string();
        self.request(ctx, "GET", &format!("/api/community/{slug}/project"), None, move |status, text| {
            let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
            let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
            match (s("title"), s("project")) {
                (Some(title), Some(json)) if status < 400 => Reply::CommunityOpened { slug: slug_, title, json },
                _ => Reply::Error(error_of(status, text)),
            }
        });
    }

    /// POST /api/community: share a project with everyone (it waits for review first)
    pub fn community_share(&mut self, ctx: &egui::Context, body: serde_json::Value) {
        self.request(ctx, "POST", "/api/community", Some(body), |status, text| {
            let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
            match v.get("url").and_then(|u| u.as_str()) {
                Some(u) if status < 400 => Reply::CommunityShared(Ok(u.to_string())),
                _ => Reply::CommunityShared(Err(error_of(status, text))),
            }
        });
    }

    pub fn fetch_shares(&mut self, ctx: &egui::Context) {
        self.request(ctx, "GET", "/api/shares", None, |status, text| {
            let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
            match v.get("shares").cloned().map(serde_json::from_value::<Vec<ShareInfo>>) {
                Some(Ok(l)) if status < 400 => Reply::Shares(l),
                _ => Reply::Error(error_of(status, text)),
            }
        });
    }

    /// accept = the data comes back (Reply::ShareTaken), else the share is turned down
    pub fn take_share(&mut self, ctx: &egui::Context, id: i64, accept: bool) {
        let (method, path) = if accept { ("POST", format!("/api/shares/{id}/accept")) } else { ("DELETE", format!("/api/shares/{id}")) };
        self.request(ctx, method, &path, if accept { Some(serde_json::json!({})) } else { None }, move |status, text| {
            if status >= 400 {
                return Reply::Error(error_of(status, text));
            }
            let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
            let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
            let data = if accept { s("data").parse::<serde_json::Value>().ok() } else { None };
            Reply::ShareTaken { kind: s("kind"), name: s("name"), data }
        });
    }

    /// Help ▸ Request a Change → POST /api/contact (the admin inbox + support@ email); signed in = the account is attached.
    /// Anonymous usage count for the admin dashboard (server usage.go): "open", "demo", "toolpath", "export".
    /// Only the word is sent, nothing about the person or the project. Web app only; fire and forget.
    pub fn event(&self, name: &str) {
        self.event_detail(name, "");
    }

    /// The same with a short detail ("Pocket", "3 files"). The server keeps the detail only for signed-in accounts
    /// (admin activity log); it never gets drawings, file names or anything typed.
    pub fn event_detail(&self, name: &str, detail: &str) {
        if !Self::web() {
            return;
        }
        let mut req = ehttp::Request::post(format!("{}/api/event", self.base), serde_json::to_vec(&serde_json::json!({ "name": name, "detail": detail })).unwrap_or_default());
        req.headers.insert("Content-Type", "application/json");
        ehttp::fetch(req, |_| {});
    }

    /// A problem the person saw (a file that would not open, a save that failed, a cloud error) goes to the site's
    /// error log (admin overview, 30 days), with a short category and the wording shown, never file names or
    /// content: callers pass messages through `scrub`. Web always; desktop only when signed in.
    pub fn report(&self, kind: &str, message: &str) {
        if kind != "crash" {
            crate::help::note_problem(message); // the status bar offers "Report this" for it
        }
        if !Self::web() && self.user.is_none() {
            return;
        }
        let place = if Self::web() { "web" } else { "desktop" };
        post_error(&self.base, &format!("{kind}: {message}"), place);
    }

    /// Leave `data` for app `to` ("gburn", "gcarve", "gsketch") in this account for 30 minutes (server handover.go);
    /// the other app opens it once with `take_handover`.
    pub fn handover(&mut self, ctx: &egui::Context, to: &str, name: &str, data: String) {
        self.request(ctx, "POST", "/api/handover", Some(serde_json::json!({"to": to, "name": name, "data": data})), |status, text| {
            let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
            Reply::HandoverSent(match v.get("id").and_then(|i| i.as_str()) {
                Some(id) if status < 400 => Ok(id.to_string()),
                _ => Err(v.get("error").and_then(|e| e.as_str()).map_or_else(|| format!("Sending failed ({status})"), String::from)),
            })
        });
    }

    pub fn take_handover(&mut self, ctx: &egui::Context, id: &str) {
        self.request(ctx, "GET", &format!("/api/handover/{id}"), None, |status, text| {
            let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
            let get = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
            if status < 400 { Reply::HandoverData { name: get("name"), data: get("data") } } else { Reply::Error(get("error")) }
        });
    }

    pub fn send_request(&mut self, ctx: &egui::Context, kind: &str, subject: &str, body: &str, email: &str) {
        let mut b = serde_json::json!({ "kind": kind, "subject": subject, "body": body, "email": email, "page": "app" });
        let done = |status: u16, text: &str| {
            let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
            Reply::RequestSent(if status < 400 {
                Ok(v.get("id").and_then(serde_json::Value::as_i64).unwrap_or(0))
            } else {
                Err(v.get("error").and_then(|e| e.as_str()).map_or_else(|| format!("Request failed ({status})"), String::from))
            })
        };
        if self.user.is_some() {
            return self.request(ctx, "POST", "/api/contact", Some(b), done);
        }
        // signed out: the server wants a proof of work (server/captcha.go): find a nonce with `bits` leading zero bits
        // in sha256(challenge:nonce). ponytail: solved inside the reply callback (~1 s; on the web that pauses the page once)
        let base = self.base.clone();
        let tx = self.tx.clone();
        let ctx2 = ctx.clone();
        self.busy += 1;
        let fail = move |tx: &std::sync::mpsc::Sender<Reply>, ctx: &egui::Context, m: String| {
            let _ = tx.send(Reply::RequestSent(Err(m)));
            ctx.request_repaint();
        };
        #[allow(unused_mut)]
        let mut req = ehttp::Request::get(format!("{base}/api/challenge"));
        #[cfg(target_arch = "wasm32")]
        {
            req.mode = ehttp::Mode::SameOrigin;
        }
        ehttp::fetch(req, move |r| {
            let v: serde_json::Value = r.ok().filter(|r| r.ok).and_then(|r| serde_json::from_slice(&r.bytes).ok()).unwrap_or_default();
            let (Some(ch), Some(bits)) = (v["challenge"].as_str(), v["bits"].as_u64()) else {
                return fail(&tx, &ctx2, "Can't reach the server right now. Check the connection and try again.".into());
            };
            let nonce = (0u64..).find(|n| pow_ok(ch, *n, bits as u32)).unwrap_or(0);
            b["challenge"] = ch.into();
            b["nonce"] = nonce.to_string().into();
            let mut post = ehttp::Request::post(format!("{base}/api/contact"), serde_json::to_vec(&b).unwrap_or_default());
            post.headers.insert("Content-Type", "application/json");
            #[cfg(target_arch = "wasm32")]
            {
                post.mode = ehttp::Mode::SameOrigin;
            }
            let (tx2, ctx3) = (tx.clone(), ctx2.clone());
            ehttp::fetch(post, move |r| {
                let reply = match r {
                    Ok(resp) => done(resp.status, resp.text().unwrap_or("")),
                    Err(e) => Reply::RequestSent(Err(format!("Can't reach the server ({e})"))),
                };
                let _ = tx2.send(reply);
                ctx3.request_repaint();
            });
        });
        // busy goes back down when its one reply is polled
    }

    /// Drain replies; returns opened/saved events for the app to act on.
    pub fn poll(&mut self, ctx: &egui::Context) -> Vec<Reply> {
        let mut events = Vec::new();
        let replies: Vec<Reply> = self.rx.try_iter().collect();
        for r in replies {
            self.busy = self.busy.saturating_sub(1);
            // a browser sign-in's answer: the wait is over, then it is an ordinary sign-in answer
            let r = match r {
                Reply::DeviceDone(_) if self.device.is_none() => continue, // cancelled in the app meanwhile
                Reply::DeviceDone(a) => {
                    self.device = None;
                    Reply::Auth(a)
                }
                other => other,
            };
            match r {
                Reply::DeviceStarted { id, code, url, secs } => {
                    ctx.open_url(egui::OpenUrl::new_tab(&url));
                    self.device = Some(Device { id, code, url, until: now() + secs, next: now() + 2, waiting: false });
                }
                Reply::DevicePending => {
                    if let Some(d) = self.device.as_mut() {
                        d.waiting = false;
                    }
                }
                Reply::DeviceFailed(e) => {
                    self.device = None;
                    self.error = e;
                }
                Reply::Auth(a) => {
                    if a.user.is_some() {
                        self.auth_gen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    self.checked = true;
                    self.need_code = a.two_factor;
                    if a.confirm {
                        self.error = "Almost done: open the link we emailed you to confirm your address, then sign in here.".into();
                    } else if let Some(e) = a.error {
                        if self.user.is_some() {
                            // session gone (expired / password changed elsewhere)
                            self.user = None;
                            self.token.clear();
                            self.store_token();
                        }
                        self.error = e;
                    } else {
                        self.user = a.user;
                        self.ent = a.entitlements.unwrap_or_default();
                        if !Self::web() {
                            // remember when the licence was last confirmed online (offline grace period)
                            crate::store::save(VERIFIED_KEY, &if self.desktop_allowed() { now().to_string() } else { String::new() });
                        }
                        if let Some(t) = a.token {
                            self.token = t;
                            self.store_token();
                        }
                        self.error.clear();
                        self.refresh(ctx);
                        self.fetch_prefs(ctx); // Qt: after sign-in / session check
                        self.fetch_library(ctx);
                        self.fetch_shares(ctx);
                    }
                }
                Reply::Expired => {
                    self.auth_gen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    self.expired = self.user.take().is_some() || self.expired;
                    self.token.clear();
                    self.store_token();
                }
                Reply::SignedOut => {
                    self.auth_gen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    crate::store::save(VERIFIED_KEY, "");
                    self.user = None;
                    self.projects.clear();
                    self.token.clear();
                    self.store_token();
                }
                Reply::Projects(p) => {
                    if p.is_empty() && self.user.is_some() {
                        // a delete answered: reload the list
                        self.projects.clear();
                    }
                    self.projects = p;
                    self.error.clear();
                }
                Reply::Error(e) => {
                    if let Some(d) = self.device.as_mut() {
                        d.waiting = false; // offline for a moment: the next poll tries again
                    }
                    self.checked = true; // offline: licensed() falls back to the grace period
                    self.report("cloud", &e);
                    self.error = e;
                }
                Reply::Config { require_signin } => self.require_signin = require_signin,
                Reply::Storage { used, max, projects, max_projects } => self.storage = Some((used, max, projects, max_projects)),
                Reply::Moved => {
                    self.error.clear();
                    self.refresh(ctx);
                }
                Reply::Saved { id, name } => {
                    self.error.clear();
                    self.refresh(ctx);
                    events.push(Reply::Saved { id, name });
                }
                other => events.push(other),
            }
        }
        self.poll_device(ctx);
        events
    }

    pub fn open_web(&self, ctx: &egui::Context, path: &str) {
        ctx.open_url(egui::OpenUrl::new_tab(format!("{}{}", self.base, path)));
    }
}

/// Days a desktop licence keeps working offline after it was last confirmed.
pub const GRACE_DAYS: i64 = 14;
const VERIFIED_KEY: &str = "license.verified";

/// A web panic, from the panic hook (no Cloud there): the page's own origin, which proxies /api to the server.
pub fn report_panic(message: &str) {
    if let Some(origin) = web_origin() {
        post_error(&origin, message, "web");
    }
}

#[cfg(target_arch = "wasm32")]
fn web_origin() -> Option<String> {
    web_sys::window()?.location().origin().ok()
}
#[cfg(not(target_arch = "wasm32"))]
fn web_origin() -> Option<String> {
    None
}

/// "GSketch v0.1.5 web": the app and its own version (both apps share this crate, so not this crate's version).
fn post_error(base: &str, message: &str, place: &str) {
    let body = serde_json::json!({ "source": "app", "message": message, "where": format!("{} v{} {place}", crate::store::app(), crate::help::version()) });
    let mut req = ehttp::Request::post(format!("{base}/api/error"), serde_json::to_vec(&body).unwrap_or_default());
    req.headers.insert("Content-Type", "application/json");
    ehttp::fetch(req, |_| {});
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
/// unix seconds of the last online confirmation, 0 = never
fn verified_at() -> i64 {
    crate::store::load(VERIFIED_KEY).and_then(|t| t.trim().parse().ok()).unwrap_or(0)
}
fn grace_days_left(at: i64, now: i64) -> i64 {
    let left = at + GRACE_DAYS * 86400 - now;
    if at > 0 && left > 0 { (left + 86399) / 86400 } else { 0 }
}
fn licensed_at(allowed_now: bool, has_token: bool, at: i64, now: i64) -> bool {
    allowed_now || has_token && grace_days_left(at, now) > 0
}

fn host_label() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "Desktop".into())
    }
    #[cfg(target_arch = "wasm32")]
    {
        "Web".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_licence_grace_period() {
        let day = 86400;
        assert!(licensed_at(true, false, 0, 1000), "Pro / admin online");
        assert!(!licensed_at(false, true, 0, 1000), "never verified");
        assert!(licensed_at(false, true, 1000, 1000 + 13 * day), "offline within 14 days");
        assert!(!licensed_at(false, false, 1000, 1000 + day), "signed out: no token, no grace");
        assert!(!licensed_at(false, true, 1000, 1000 + 14 * day), "grace over");
        assert_eq!(grace_days_left(1000, 1000 + day / 2), 14);
        assert_eq!(grace_days_left(1000, 1000 + 13 * day + 1), 1);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod live {
    //! Runs against a real gcarve-api when GCARVE_CLOUD_URL and GCARVE_TEST_INVITE are set:
    //!   cargo test -p gcarve-app cloud -- --ignored
    use super::*;
    use std::time::{Duration, Instant};

    fn pump(c: &mut Cloud, ctx: &egui::Context, until: impl Fn(&Cloud) -> bool) -> Vec<Reply> {
        let start = Instant::now();
        let mut ev = Vec::new();
        while start.elapsed() < Duration::from_secs(10) {
            ev.extend(c.poll(ctx));
            if until(c) && c.busy == 0 {
                return ev;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        panic!("timed out: error={} busy={}", c.error, c.busy);
    }

    #[test]
    #[ignore]
    fn register_save_open_signout() {
        let Ok(invite) = std::env::var("GCARVE_TEST_INVITE") else { return };
        let ctx = egui::Context::default();
        let mut c = Cloud::new();
        c.token.clear();
        c.register(&ctx, "rust@example.com", "correct-horse-battery", &invite);
        pump(&mut c, &ctx, |c| c.signed_in());
        assert_eq!(c.user.as_ref().unwrap().email, "rust@example.com");
        assert_eq!(c.ent.plan, "free");
        c.save_project(&ctx, "", "Rust test", None, r#"{"app":"GCarve","version":3}"#);
        let ev = pump(&mut c, &ctx, |c| !c.projects.is_empty());
        let id = ev.iter().find_map(|e| if let Reply::Saved { id, .. } = e { Some(id.clone()) } else { None }).expect("saved");
        c.open_project(&ctx, &id);
        let ev = pump(&mut c, &ctx, |_| true);
        assert!(ev.iter().any(|e| matches!(e, Reply::Opened { json, .. } if json.contains("GCarve"))));
        c.sign_out(&ctx);
        pump(&mut c, &ctx, |c| !c.signed_in());
        assert!(c.load_token().is_empty(), "token must be forgotten on sign-out");
    }

    /// the desktop signs in through the browser (an account with any sign-in), then lists and opens its models
    #[test]
    #[ignore]
    fn browser_sign_in_then_open_a_model() {
        let Ok(invite) = std::env::var("GCARVE_TEST_INVITE3") else { return };
        let base = std::env::var("GCARVE_CLOUD_URL").unwrap();
        let send = |method: &str, path: &str, ctype: &str, body: String, cookie: &str| {
            let mut r = ehttp::Request::post(format!("{base}{path}"), body.into_bytes());
            r.method = method.into();
            r.headers.insert("Content-Type", ctype);
            r.headers.insert("Origin", base.clone());
            if !cookie.is_empty() {
                r.headers.insert("Cookie", cookie);
            }
            ehttp::fetch_blocking(&r).expect("server")
        };
        // the browser: an account signed in on the website (a cookie)
        let reg = send("POST", "/api/auth/register", "application/json", serde_json::json!({"email": "browser@example.com", "password": "correct-horse-battery", "invite": invite, "client": "web"}).to_string(), "");
        assert_eq!(reg.status, 200, "{:?}", reg.text());
        let cookie = reg.headers.get("set-cookie").expect("session cookie").split(';').next().unwrap().to_string();
        // the desktop asks; the browser page shows the same code and allows it
        let ctx = egui::Context::default();
        let mut c = Cloud::new();
        c.token.clear();
        c.docs = "sketches";
        c.browser_sign_in(&ctx);
        pump(&mut c, &ctx, |c| c.device.is_some());
        let code = c.device.as_ref().unwrap().code.clone();
        let page = send("GET", &format!("/api/auth/device?code={code}"), "text/plain", String::new(), &cookie);
        assert!(page.status == 200 && page.text().unwrap_or("").contains(&code), "page: {}", page.status);
        let ok = send("POST", "/api/auth/device/allow", "application/x-www-form-urlencoded", format!("ok=1&code={code}"), &cookie);
        assert_eq!(ok.status, 200);
        pump(&mut c, &ctx, |c| c.signed_in());
        assert_eq!(c.user.as_ref().unwrap().email, "browser@example.com");
        assert!(c.device.is_none() && !c.load_token().is_empty(), "a desktop token is kept");
        c.save_project(&ctx, "", "From the desktop", None, r#"{"app":"GSketch","version":2,"mesh":{}}"#);
        pump(&mut c, &ctx, |c| c.projects.iter().any(|p| p.name == "From the desktop"));
        c.sign_out(&ctx);
        pump(&mut c, &ctx, |c| !c.signed_in());
    }

    /// two accounts (GCARVE_TEST_INVITE + GCARVE_TEST_INVITE2): library round trip, then A shares bits with B
    #[test]
    #[ignore]
    fn library_and_sharing() {
        let (Ok(i1), Ok(i2)) = (std::env::var("GCARVE_TEST_INVITE"), std::env::var("GCARVE_TEST_INVITE2")) else { return };
        let ctx = egui::Context::default();
        let (mut a, mut b) = (Cloud::new(), Cloud::new());
        a.token.clear();
        b.token.clear();
        a.register(&ctx, "share-a@example.com", "correct-horse-battery", &i1);
        pump(&mut a, &ctx, |c| c.signed_in());
        let bits = serde_json::json!([{ "name": "Shared 1/8 ball", "diameter": 0.125 }]);
        a.push_library(&ctx, "bits", bits.clone());
        pump(&mut a, &ctx, |_| true);
        a.fetch_library(&ctx);
        let ev = pump(&mut a, &ctx, |_| true);
        assert!(ev.iter().any(|e| matches!(e, Reply::Library { bits: Some(v), .. } if v.to_string().contains("Shared 1/8 ball"))), "library back");
        a.share(&ctx, "share-b@example.com", "bits", "My ball bit", bits);
        let ev = pump(&mut a, &ctx, |_| true);
        assert!(ev.iter().any(|e| matches!(e, Reply::Shared(_))), "shared: {}", a.error);
        b.register(&ctx, "share-b@example.com", "correct-horse-battery", &i2);
        let mut ev = pump(&mut b, &ctx, |c| c.signed_in());
        ev.extend(pump(&mut b, &ctx, |_| true)); // sign-in fetches the shares
        let id = ev.iter().find_map(|e| if let Reply::Shares(l) = e { l.first().map(|s| s.id) } else { None }).expect("b sees the share");
        b.take_share(&ctx, id, true);
        let ev = pump(&mut b, &ctx, |_| true);
        let data = ev.iter().find_map(|e| match e {
            Reply::ShareTaken { data: Some(d), kind, .. } if kind == "bits" => Some(d.clone()),
            _ => None,
        });
        let data = data.expect("accepted with data");
        assert!(data.to_string().contains("Shared 1/8 ball"));
    }
}

/// sha256(challenge + ":" + nonce) starts with `bits` zero bits
fn pow_ok(challenge: &str, nonce: u64, bits: u32) -> bool {
    use sha2::{Digest, Sha256};
    let h = Sha256::digest(format!("{challenge}:{nonce}").as_bytes());
    let (bytes, rest) = ((bits / 8) as usize, bits % 8);
    h[..bytes].iter().all(|b| *b == 0) && (rest == 0 || h[bytes] >> (8 - rest) == 0)
}

#[cfg(test)]
mod pow_tests {
    #[test]
    fn nonce_search_meets_the_difficulty() {
        let n = (0u64..).find(|n| super::pow_ok("abc", *n, 12)).unwrap();
        use sha2::{Digest, Sha256};
        let h = Sha256::digest(format!("abc:{n}").as_bytes());
        assert!(h[0] == 0 && h[1] >> 4 == 0);
    }
}
