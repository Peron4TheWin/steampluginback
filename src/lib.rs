use std::thread;
use std::sync::OnceLock;
use std::io::Read;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use tiny_http::{Server, Response, Header};

type FnGetFileVersionInfoA = unsafe extern "system" fn(*const i8, u32, u32, *mut u8) -> i32;
type FnGetFileVersionInfoW = unsafe extern "system" fn(*const u16, u32, u32, *mut u8) -> i32;
type FnGetFileVersionInfoSizeA = unsafe extern "system" fn(*const i8, *mut u32) -> u32;
type FnGetFileVersionInfoSizeW = unsafe extern "system" fn(*const u16, *mut u32) -> u32;
type FnGetFileVersionInfoSizeExW = unsafe extern "system" fn(u32, *const u16, *mut u32) -> u32;
type FnGetFileVersionInfoExW = unsafe extern "system" fn(u32, *const u16, u32, u32, *mut u8) -> i32;
type FnVerFindFileA = unsafe extern "system" fn(u32, *const i8, *const i8, *const i8, *mut i8, *mut u32, *mut i8, *mut u32) -> u32;
type FnVerFindFileW = unsafe extern "system" fn(u32, *const u16, *const u16, *const u16, *mut u16, *mut u32, *mut u16, *mut u32) -> u32;
type FnVerInstallFileA = unsafe extern "system" fn(u32, *const i8, *const i8, *const i8, *const i8, *const i8, *mut i8, *mut u32) -> u32;
type FnVerInstallFileW = unsafe extern "system" fn(u32, *const u16, *const u16, *const u16, *const u16, *const u16, *mut u16, *mut u32) -> u32;
type FnVerLanguageNameA = unsafe extern "system" fn(u32, *mut i8, u32) -> u32;
type FnVerLanguageNameW = unsafe extern "system" fn(u32, *mut u16, u32) -> u32;
type FnVerQueryValueA = unsafe extern "system" fn(*const u8, *const i8, *mut *mut u8, *mut u32) -> i32;
type FnVerQueryValueW = unsafe extern "system" fn(*const u8, *const u16, *mut *mut u8, *mut u32) -> i32;

struct VersionFns {
    GetFileVersionInfoA: FnGetFileVersionInfoA,
    GetFileVersionInfoW: FnGetFileVersionInfoW,
    GetFileVersionInfoSizeA: FnGetFileVersionInfoSizeA,
    GetFileVersionInfoSizeW: FnGetFileVersionInfoSizeW,
    GetFileVersionInfoSizeExW: FnGetFileVersionInfoSizeExW,
    GetFileVersionInfoExW: FnGetFileVersionInfoExW,
    VerFindFileA: FnVerFindFileA,
    VerFindFileW: FnVerFindFileW,
    VerInstallFileA: FnVerInstallFileA,
    VerInstallFileW: FnVerInstallFileW,
    VerLanguageNameA: FnVerLanguageNameA,
    VerLanguageNameW: FnVerLanguageNameW,
    VerQueryValueA: FnVerQueryValueA,
    VerQueryValueW: FnVerQueryValueW,
}

unsafe impl Send for VersionFns {}
unsafe impl Sync for VersionFns {}

static REAL: OnceLock<VersionFns> = OnceLock::new();

extern "system" {
    fn LoadLibraryA(name: *const i8) -> *mut u8;
    fn GetProcAddress(module: *mut u8, name: *const i8) -> *mut u8;
    fn GetModuleFileNameW(hmodule: *mut u8, filename: *mut u16, size: u32) -> u32;
    fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
    fn ShellExecuteA(hwnd: *mut u8, op: *const i8, file: *const i8, params: *const i8, dir: *const i8, show: i32) -> *mut u8;
}

// ── Paths ─────────────────────────────────────────────────────────────────────

static STEAM_DIR: OnceLock<String> = OnceLock::new();
static SYSTEM32_DIR: OnceLock<String> = OnceLock::new();
static OWN_PATH: OnceLock<String> = OnceLock::new();
#[link(name = "shell32")]
unsafe extern "system" {}
fn init_paths(hmodule: *mut u8) {
    unsafe {
        let mut sys_buf = vec![0u16; 260];
        let sys_len = GetSystemDirectoryW(sys_buf.as_mut_ptr(), 260);
        let system32 = if sys_len > 0 {
            sys_buf.truncate(sys_len as usize);
            OsString::from_wide(&sys_buf).to_string_lossy().to_string()
        } else {
            std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string()) + "\\System32"
        };
        SYSTEM32_DIR.set(system32).ok();

        let mut dll_buf = vec![0u16; 260];
        let dll_len = GetModuleFileNameW(hmodule, dll_buf.as_mut_ptr(), 260);
        if dll_len > 0 {
            dll_buf.truncate(dll_len as usize);
            let full = OsString::from_wide(&dll_buf).to_string_lossy().to_string();
            OWN_PATH.set(full.clone()).ok();
            let steam_dir = full.rfind('\\')
                .map(|i| full[..i].to_string())
                .unwrap_or_else(|| ".".to_string());
            STEAM_DIR.set(steam_dir).ok();
        } else {
            OWN_PATH.set(".\\version.dll".to_string()).ok();
            let cwd = std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string());
            STEAM_DIR.set(cwd).ok();
        }
    }
}

fn steam_dir() -> &'static str { STEAM_DIR.get().map(|s| s.as_str()).unwrap_or(".") }
fn system32_dir() -> &'static str { SYSTEM32_DIR.get().map(|s| s.as_str()).unwrap_or("C:\\Windows\\System32") }
fn own_path() -> &'static str { OWN_PATH.get().map(|s| s.as_str()).unwrap_or(".\\version.dll") }
fn lua_dir() -> String { format!("{}\\config\\stplug-in", steam_dir()) }
fn key_path() -> String { format!("{}\\key.txt", steam_dir()) }
fn log_path() -> String { format!("{}\\backend.log", steam_dir()) }
fn real_version_dll_path() -> String { format!("{}\\version.dll", system32_dir()) }
fn cloudredirect_exe_path() -> String { format!("{}\\CloudRedirectCLI.exe", steam_dir()) }
fn script_path() -> String { format!("{}\\content.js", steam_dir()) }

// ── Logger ────────────────────────────────────────────────────────────────────

use std::sync::Mutex;
static LOG: OnceLock<Mutex<std::fs::File>> = OnceLock::new();

fn init_log() {
    let file = std::fs::OpenOptions::new()
        .create(true).append(true)
        .open(log_path())
        .expect("Failed to open backend.log");
    LOG.set(Mutex::new(file)).ok();
    log("============================================================");
    log("DLL loaded");
    log(&format!("Steam dir:   {}", steam_dir()));
    log(&format!("System32:    {}", system32_dir()));
    log(&format!("Own path:    {}", own_path()));
    log(&format!("lua dir:     {}", lua_dir()));
    log(&format!("key file:    {}", key_path()));
}

fn log(msg: &str) {
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    if let Some(f) = LOG.get() {
        if let Ok(mut f) = f.lock() {
            writeln!(f, "[{}] {}", ts, msg).ok();
        }
    }
}

// ── SHA256 ────────────────────────────────────────────────────────────────────

fn sha256_of_bytes(data: &[u8]) -> String {
    use sha2::{Sha256, Digest};
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

fn sha256_of_file(path: &str) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    Some(sha256_of_bytes(&data))
}

// ── GitHub release parser ─────────────────────────────────────────────────────

// Devuelve (tag, download_url, sha256) sacando el digest directo del JSON
fn github_latest_asset(repo: &str, asset_name: &str) -> Option<(String, String, String)> {
    let api_url = format!("https://api.github.com/repos/{}/releases/latest", repo);
    log(&format!("Checking GitHub latest for {} (asset: {})...", repo, asset_name));

    let resp = ureq::get(&api_url)
        .set("User-Agent", "steampluginback/1.0")
        .set("Accept", "application/vnd.github+json")
        .call().ok()?;

    let body = resp.into_string().ok()?;
    log(&format!("GitHub API response length: {} bytes", body.len()));

    let tag = extract_json_str(&body, "tag_name")?;

    let (download_url, digest) = find_asset_in_array(&body, asset_name)?;

    // digest viene como "sha256:abc123..." — sacar solo el hash
    let sha256 = digest.strip_prefix("sha256:").unwrap_or(&digest).to_string();

    log(&format!("Latest tag: {}, asset sha256: {}...", tag, &sha256[..sha256.len().min(16)]));
    Some((tag, download_url, sha256))
}

fn extract_json_str(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\":", key);
    let start = json.find(&needle)? + needle.len();
    let rest = json[start..].trim_start();
    if rest.starts_with('"') {
        let rest = &rest[1..];
        let end = rest.find('"')?;
        Some(rest[..end].to_string())
    } else {
        None
    }
}

fn find_asset_in_array(json: &str, asset_name: &str) -> Option<(String, String)> {
    // Buscar a partir del array "assets"
    let assets_pos = json.find("\"assets\":")?;
    let assets_json = &json[assets_pos..];

    // Buscar "name":"<asset_name>" dentro del array
    let needle = format!("\"name\":\"{}\"", asset_name);
    let pos = assets_json.find(&needle)?;

    // Retroceder para encontrar el { de inicio del objeto de este asset
    let before = &assets_json[..pos];
    let obj_start = before.rfind('{')?;
    let obj = &assets_json[obj_start..];

    // Delimitar el objeto
    let obj_end = find_object_end(obj)?;
    let obj_slice = &obj[..obj_end];

    let download_url = extract_json_str(obj_slice, "browser_download_url")?;
    let digest = extract_json_str(obj_slice, "digest").unwrap_or_default();

    Some((download_url, digest))
}

fn find_object_end(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    for (i, c) in s.char_indices() {
        if escape { escape = false; continue; }
        if in_string {
            if c == '\\' { escape = true; }
            else if c == '"' { in_string = false; }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 { return Some(i + 1); }
            }
            _ => {}
        }
    }
    None
}

fn download_bytes(url: &str) -> Option<Vec<u8>> {
    log(&format!("Downloading: {}", url));
    let resp = ureq::get(url)
        .set("User-Agent", "steampluginback/1.0")
        .call().ok()?;
    let mut buf = Vec::new();
    resp.into_reader().read_to_end(&mut buf).ok()?;
    log(&format!("Downloaded {} bytes", buf.len()));
    Some(buf)
}

// ── Auto-update: CloudRedirectCLI ─────────────────────────────────────────────

fn update_cloudredirect() {
    log("=== CloudRedirect update check ===");
    let exe_path = cloudredirect_exe_path();
    let stamp_path = format!("{}\\CloudRedirectCLI.stamp", steam_dir());

    let (_, download_url, remote_hash) = match github_latest_asset("Selectively11/CloudRedirect", "CloudRedirectCLI.exe") {
        Some(v) => v,
        None => { log("WARN: Could not fetch CloudRedirect release info"); return; }
    };

    // Leer el stamp (hash de la última versión ejecutada)
    let last_run_hash = std::fs::read_to_string(&stamp_path).unwrap_or_default().trim().to_string();

    if std::path::Path::new(&exe_path).exists() {
        let local_hash = sha256_of_file(&exe_path).unwrap_or_default();
        log(&format!("Local  SHA256: {}", local_hash));
        log(&format!("Remote SHA256: {}", remote_hash));
        log(&format!("Last run SHA256: {}", last_run_hash));

        if local_hash == remote_hash && last_run_hash == remote_hash {
            log("CloudRedirectCLI is up to date and already ran — skipping");
            return;
        }

        if local_hash != remote_hash {
            log("SHA256 mismatch — updating...");
            let bytes = match download_bytes(&download_url) {
                Some(b) => b,
                None => { log("ERROR: Failed to download CloudRedirectCLI.exe"); return; }
            };
            if let Err(e) = std::fs::write(&exe_path, &bytes) {
                log(&format!("ERROR: Failed to write CloudRedirectCLI.exe: {}", e));
                return;
            }
            log("CloudRedirectCLI.exe updated");
        }
    } else {
        log("CloudRedirectCLI.exe not found — downloading...");
        let bytes = match download_bytes(&download_url) {
            Some(b) => b,
            None => { log("ERROR: Failed to download CloudRedirectCLI.exe"); return; }
        };
        if let Err(e) = std::fs::write(&exe_path, &bytes) {
            log(&format!("ERROR: Failed to write CloudRedirectCLI.exe: {}", e));
            return;
        }
        log("CloudRedirectCLI.exe downloaded");
    }

    run_cloudredirect(&exe_path);

    // Guardar el hash de la versión ejecutada
    std::fs::write(&stamp_path, &remote_hash).ok();
    log(&format!("Stamp updated: {}", stamp_path));
}

fn run_cloudredirect(exe_path: &str) {
    log(&format!("Running: {} /stfixer", exe_path));
    match std::process::Command::new(exe_path)
        .arg("/stfixer")
        .spawn()
    {
        Ok(mut child) => {
            log("CloudRedirectCLI launched, waiting for exit...");
            match child.wait() {
                Ok(status) => log(&format!("CloudRedirectCLI exited: {}", status)),
                Err(e) => log(&format!("ERROR waiting for CloudRedirectCLI: {}", e)),
            }
        }
        Err(e) => log(&format!("ERROR: Failed to run CloudRedirectCLI: {}", e)),
    }
}

// ── Auto-update: version.dll (self) ──────────────────────────────────────────

static DLL_UPDATE_PENDING: OnceLock<Mutex<bool>> = OnceLock::new();

fn update_self() {
    log("=== version.dll self-update check ===");
    DLL_UPDATE_PENDING.set(Mutex::new(false)).ok();

    let (_, download_url, remote_hash) = match github_latest_asset("Peron4TheWin/steampluginback", "version.dll") {
        Some(v) => v,
        None => { log("WARN: Could not fetch version.dll release info"); return; }
    };

    let local_hash = sha256_of_file(own_path()).unwrap_or_default();
    log(&format!("Local  SHA256: {}", local_hash));
    log(&format!("Remote SHA256: {}", remote_hash));

    if local_hash == remote_hash {
        log("version.dll is up to date");
        return;
    }

    log("version.dll update available — downloading...");
    let bytes = match download_bytes(&download_url) {
        Some(b) => b,
        None => { log("ERROR: Failed to download latest version.dll"); return; }
    };

    let new_path = format!("{}\\version_new.dll", steam_dir());
    if let Err(e) = std::fs::write(&new_path, &bytes) {
        log(&format!("ERROR: Failed to write version_new.dll: {}", e));
        return;
    }
    log(&format!("Saved new DLL to: {}", new_path));

    // PowerShell que espera que steam muera, mueve el dll y relanza Steam
    let own = own_path().replace('\\', "\\\\");
    let new = new_path.replace('\\', "\\\\");
    let steam_exe = format!("{}\\steam.exe", steam_dir()).replace('\\', "\\\\");

    let ps_script = format!(
        "$src = \"{new}\"\n\
         $dst = \"{own}\"\n\
         $steamExe = \"{steam_exe}\"\n\
         Write-Host \"Waiting for Steam to exit...\"\n\
         while (Get-Process -Name \"steam\" -ErrorAction SilentlyContinue) {{\n\
             Start-Sleep -Milliseconds 500\n\
         }}\n\
         Start-Sleep -Milliseconds 500\n\
         Move-Item -Force $src $dst\n\
         Write-Host \"version.dll updated\"\n\
         Start-Process $steamExe\n\
         Write-Host \"Steam relaunched\"\n",
        new = new, own = own, steam_exe = steam_exe
    );

    let ps_path = format!("{}\\update_dll.ps1", steam_dir());
    if let Err(e) = std::fs::write(&ps_path, &ps_script) {
        log(&format!("ERROR: Failed to write update_dll.ps1: {}", e));
        return;
    }
    log(&format!("PowerShell script saved: {}", ps_path));

    // Lanzar PS en background invisible
    unsafe {
        let mut ps_arg: Vec<u8> = format!(
            "-ExecutionPolicy Bypass -WindowStyle Hidden -File \"{}\"\0", ps_path
        ).bytes().collect();
        ShellExecuteA(
            std::ptr::null_mut(),
            b"open\0".as_ptr() as _,
            b"powershell.exe\0".as_ptr() as _,
            ps_arg.as_ptr() as _,
            std::ptr::null(),
            0,
        );
    }
    log("PowerShell updater launched — killing Steam...");

    std::process::Command::new("taskkill")
        .args(["/F", "/IM", "steam.exe"])
        .spawn().ok();

    log("Steam kill signal sent — PS will relaunch it after move");

    if let Some(flag) = DLL_UPDATE_PENDING.get() {
        if let Ok(mut f) = flag.lock() { *f = true; }
    }
}

fn dll_update_pending() -> bool {
    DLL_UPDATE_PENDING.get()
        .and_then(|m| m.lock().ok())
        .map(|f| *f)
        .unwrap_or(false)
}

// ── Auto-update: content.js ───────────────────────────────────────────────────

fn update_script() {
    log("=== content.js update check ===");
    let url = "https://raw.githubusercontent.com/Peron4TheWin/steampluginfront/refs/heads/master/content/content.js";
    let local_path = script_path();

    let resp = match ureq::get(url).set("User-Agent", "steampluginback/1.0").call() {
        Ok(r) => r,
        Err(e) => { log(&format!("WARN: Failed to fetch content.js: {} — using cache", e)); return; }
    };

    if resp.status() != 200 {
        log(&format!("WARN: content.js fetch returned {} — keeping local", resp.status()));
        return;
    }

    let mut remote_bytes = Vec::new();
    if resp.into_reader().read_to_end(&mut remote_bytes).is_err() {
        log("ERROR: Failed to read content.js body");
        return;
    }

    let remote_hash = sha256_of_bytes(&remote_bytes);
    let local_hash = sha256_of_file(&local_path).unwrap_or_default();

    if local_hash == remote_hash {
        log("content.js is up to date");
        return;
    }

    log("content.js changed — updating...");
    match std::fs::write(&local_path, &remote_bytes) {
        Ok(_) => log(&format!("content.js saved to {}", local_path)),
        Err(e) => log(&format!("ERROR: Failed to write content.js: {}", e)),
    }
}

// ── Version DLL loader ────────────────────────────────────────────────────────

fn load_real_version() {
    unsafe {
        let path = real_version_dll_path();
        log(&format!("Loading real version.dll from {}...", path));
        let mut path_bytes: Vec<u8> = path.bytes().collect();
        path_bytes.push(0);
        let lib = LoadLibraryA(path_bytes.as_ptr() as _);
        if lib.is_null() {
            log("ERROR: Failed to load real version.dll");
            return;
        }

        macro_rules! get_fn {
            ($name:ident) => {{
                let ptr = GetProcAddress(lib, concat!(stringify!($name), "\0").as_ptr() as _);
                if ptr.is_null() { log(&format!("WARN: GetProcAddress null for {}", stringify!($name))); }
                std::mem::transmute(ptr)
            }};
        }

        REAL.set(VersionFns {
            GetFileVersionInfoA: get_fn!(GetFileVersionInfoA),
            GetFileVersionInfoW: get_fn!(GetFileVersionInfoW),
            GetFileVersionInfoSizeA: get_fn!(GetFileVersionInfoSizeA),
            GetFileVersionInfoSizeW: get_fn!(GetFileVersionInfoSizeW),
            GetFileVersionInfoSizeExW: get_fn!(GetFileVersionInfoSizeExW),
            GetFileVersionInfoExW: get_fn!(GetFileVersionInfoExW),
            VerFindFileA: get_fn!(VerFindFileA),
            VerFindFileW: get_fn!(VerFindFileW),
            VerInstallFileA: get_fn!(VerInstallFileA),
            VerInstallFileW: get_fn!(VerInstallFileW),
            VerLanguageNameA: get_fn!(VerLanguageNameA),
            VerLanguageNameW: get_fn!(VerLanguageNameW),
            VerQueryValueA: get_fn!(VerQueryValueA),
            VerQueryValueW: get_fn!(VerQueryValueW),
        }).ok();
        log("version.dll exports loaded OK");
    }
}

// ── DllMain ───────────────────────────────────────────────────────────────────

fn is_primary_process() -> bool {
    // Si ya hay algo en el puerto, somos un proceso secundario de Steam
    std::net::TcpStream::connect("127.0.0.1:3000").is_err()
}

#[no_mangle]
pub extern "system" fn DllMain(hmodule: *mut u8, reason: u32, _reserved: *mut u8) -> i32 {
    match reason {
        1 => {
            init_paths(hmodule);
            init_log();
            load_real_version();
            init_api_key();

            if is_primary_process() {
                log("Primary process — starting server and updates");
                thread::spawn(|| {
                    update_cloudredirect();
                    update_self();
                    update_script();
                });
                thread::spawn(|| run_server());
            } else {
                log("Secondary Steam process — skipping server and updates");
            }
        }
        0 => log("DLL_PROCESS_DETACH"),
        _ => {}
    }
    1
}

// ── API Key ───────────────────────────────────────────────────────────────────

use std::fs;
use std::path::Path;
use std::sync::RwLock;

static API_KEY: OnceLock<RwLock<String>> = OnceLock::new();

fn init_api_key() {
    let path = key_path();
    log(&format!("Loading API key from '{}'", path));
    if !Path::new(&path).exists() {
        log("key.txt not found, creating empty");
        fs::write(&path, "").expect("Failed to create key.txt");
    }
    let key = fs::read_to_string(&path).expect("Failed to read key.txt").trim().to_string();
    if key.is_empty() {
        log("WARN: key.txt is empty");
    } else {
        log(&format!("API key loaded ({}...)", &key[..key.len().min(8)]));
    }
    API_KEY.set(RwLock::new(key)).ok();
}

fn get_api_key() -> String { API_KEY.get().unwrap().read().unwrap().clone() }

fn set_api_key(new_key: String) {
    log(&format!("Updating API key ({}...)", &new_key[..new_key.len().min(8)]));
    { *API_KEY.get().unwrap().write().unwrap() = new_key.clone(); }
    fs::write(key_path(), new_key).expect("Failed to save key");
    log("API key saved");
}

fn cors_header() -> Header {
    Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap()
}

// ── Server ────────────────────────────────────────────────────────────────────

fn run_server() {
    log("Starting HTTP server on 127.0.0.1:3000...");
    let server = match Server::http("127.0.0.1:3000") {
        Ok(s) => { log("Server listening on 127.0.0.1:3000"); s }
        Err(e) => { log(&format!("ERROR: Failed to bind: {}", e)); return; }
    };

    for mut request in server.incoming_requests() {
        let method = request.method().as_str().to_string();
        let url = request.url().to_string();
        log(&format!("--> {} {}", method, url));

        // GET /script
        if method == "GET" && url.trim_start_matches('/').starts_with("script"){
            thread::spawn(|| update_script());
            let local = script_path();
            match std::fs::read(&local) {
                Ok(bytes) => {
                    let response = Response::from_data(bytes)
                        .with_header(cors_header())
                        .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/javascript"[..]).unwrap());
                    request.respond(response).ok();
                    log("<-- 200 /script (cached)");
                }
                Err(_) => {
                    log("No local content.js — fetching synchronously...");
                    update_script();
                    match std::fs::read(&local) {
                        Ok(bytes) => {
                            let response = Response::from_data(bytes)
                                .with_header(cors_header())
                                .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/javascript"[..]).unwrap());
                            request.respond(response).ok();
                            log("<-- 200 /script (first fetch)");
                        }
                        Err(_) => {
                            request.respond(Response::from_string("Script not available").with_status_code(503).with_header(cors_header())).ok();
                            log("<-- 503 /script");
                        }
                    }
                }
            }
            continue;
        }

        // GET /status
        if method == "GET" && url.trim_start_matches('/') == "status" {
            let body = format!(
                "{{\"update_pending\":{},\"key_set\":{}}}",
                dll_update_pending(),
                !get_api_key().is_empty()
            );
            let response = Response::from_string(body)
                .with_header(cors_header())
                .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
            request.respond(response).ok();
            log("<-- 200 /status");
            continue;
        }

        if method != "POST" {
            request.respond(Response::from_string("Method Not Allowed").with_status_code(405)).ok();
            log("<-- 405");
            continue;
        }

        // POST /key
        if url.trim_start_matches('/') == "key" {
            let mut body = String::new();
            if request.as_reader().read_to_string(&mut body).is_err() {
                request.respond(Response::from_string("Failed to read body").with_status_code(400)).ok();
                continue;
            }
            let key = body.trim().to_string();
            log(&format!("Validating key ({}...)", &key[..key.len().min(8)]));

            match ureq::get("https://hubcapmanifest.com/api/v1/user/stats")
                .set("Authorization", &format!("Bearer {}", key))
                .call()
            {
                Ok(_) => {
                    log("Key OK");
                    set_api_key(key);
                    request.respond(Response::from_string("OK").with_status_code(200).with_header(cors_header())).ok();
                    log("<-- 200 /key");
                }
                Err(ureq::Error::Status(401, resp)) => {
                    let msg = resp.into_string().unwrap_or_else(|_| "Unauthorized".to_string());
                    log(&format!("Key invalid: {}", msg));
                    request.respond(Response::from_string(msg).with_status_code(401).with_header(cors_header())).ok();
                    log("<-- 401 /key");
                }
                Err(err) => {
                    log(&format!("Key error: {}", err));
                    request.respond(Response::from_string(format!("API error: {}", err)).with_status_code(500).with_header(cors_header())).ok();
                    log("<-- 500 /key");
                }
            }
            continue;
        }

        // POST /<appid>
        let appid = url.trim_start_matches('/').to_string();
        log(&format!("Fetching lua for appid: {}", appid));
        let mut _body = String::new();
        request.as_reader().read_to_string(&mut _body).ok();

        let fetch_url = format!("https://hubcapmanifest.com/api/v1/lua/{}", appid);
        log(&format!("GET {}", fetch_url));

        match ureq::get(&fetch_url)
            .set("Authorization", &format!("Bearer {}", get_api_key()))
            .call()
        {
            Ok(resp) => {
                log(&format!("hubcapmanifest -> {}", resp.status()));
                let mut buf = Vec::new();
                resp.into_reader().read_to_end(&mut buf).ok();
                log(&format!("Received {} bytes", buf.len()));
                std::fs::create_dir_all(lua_dir()).ok();
                let path = format!("{}\\{}.lua", lua_dir(), appid);
                match std::fs::write(&path, &buf) {
                    Ok(_) => log(&format!("Saved: {}", path)),
                    Err(e) => log(&format!("ERROR writing {}: {}", path, e)),
                }
                request.respond(Response::from_string("OK").with_header(cors_header())).ok();
                log("<-- 200 OK");
            }
            Err(ureq::Error::Status(code, resp)) => {
                let msg = resp.into_string().unwrap_or_default();
                log(&format!("ERROR: {} {}", code, msg));
                request.respond(Response::from_string(msg).with_status_code(code).with_header(cors_header())).ok();
                log(&format!("<-- {}", code));
            }
            Err(err) => {
                log(&format!("ERROR: {}", err));
                request.respond(Response::from_string(err.to_string()).with_status_code(500).with_header(cors_header())).ok();
                log("<-- 500");
            }
        }
    }
}

// ── Exports ───────────────────────────────────────────────────────────────────

#[no_mangle] pub unsafe extern "system" fn GetFileVersionInfoA(a: *const i8, b: u32, c: u32, d: *mut u8) -> i32 { (REAL.get().unwrap().GetFileVersionInfoA)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn GetFileVersionInfoW(a: *const u16, b: u32, c: u32, d: *mut u8) -> i32 { (REAL.get().unwrap().GetFileVersionInfoW)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn GetFileVersionInfoSizeA(a: *const i8, b: *mut u32) -> u32 { (REAL.get().unwrap().GetFileVersionInfoSizeA)(a, b) }
#[no_mangle] pub unsafe extern "system" fn GetFileVersionInfoSizeW(a: *const u16, b: *mut u32) -> u32 { (REAL.get().unwrap().GetFileVersionInfoSizeW)(a, b) }
#[no_mangle] pub unsafe extern "system" fn GetFileVersionInfoSizeExW(a: u32, b: *const u16, c: *mut u32) -> u32 { (REAL.get().unwrap().GetFileVersionInfoSizeExW)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn GetFileVersionInfoExW(a: u32, b: *const u16, c: u32, d: u32, e: *mut u8) -> i32 { (REAL.get().unwrap().GetFileVersionInfoExW)(a, b, c, d, e) }
#[no_mangle] pub unsafe extern "system" fn VerFindFileA(a: u32, b: *const i8, c: *const i8, d: *const i8, e: *mut i8, f: *mut u32, g: *mut i8, h: *mut u32) -> u32 { (REAL.get().unwrap().VerFindFileA)(a, b, c, d, e, f, g, h) }
#[no_mangle] pub unsafe extern "system" fn VerFindFileW(a: u32, b: *const u16, c: *const u16, d: *const u16, e: *mut u16, f: *mut u32, g: *mut u16, h: *mut u32) -> u32 { (REAL.get().unwrap().VerFindFileW)(a, b, c, d, e, f, g, h) }
#[no_mangle] pub unsafe extern "system" fn VerInstallFileA(a: u32, b: *const i8, c: *const i8, d: *const i8, e: *const i8, f: *const i8, g: *mut i8, h: *mut u32) -> u32 { (REAL.get().unwrap().VerInstallFileA)(a, b, c, d, e, f, g, h) }
#[no_mangle] pub unsafe extern "system" fn VerInstallFileW(a: u32, b: *const u16, c: *const u16, d: *const u16, e: *const u16, f: *const u16, g: *mut u16, h: *mut u32) -> u32 { (REAL.get().unwrap().VerInstallFileW)(a, b, c, d, e, f, g, h) }
#[no_mangle] pub unsafe extern "system" fn VerLanguageNameA(a: u32, b: *mut i8, c: u32) -> u32 { (REAL.get().unwrap().VerLanguageNameA)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn VerLanguageNameW(a: u32, b: *mut u16, c: u32) -> u32 { (REAL.get().unwrap().VerLanguageNameW)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn VerQueryValueA(a: *const u8, b: *const i8, c: *mut *mut u8, d: *mut u32) -> i32 { (REAL.get().unwrap().VerQueryValueA)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn VerQueryValueW(a: *const u8, b: *const u16, c: *mut *mut u8, d: *mut u32) -> i32 { (REAL.get().unwrap().VerQueryValueW)(a, b, c, d) }