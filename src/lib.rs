use std::thread;
use std::sync::OnceLock;
use std::io::Read;
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
}

// ── Logger ────────────────────────────────────────────────────────────────────

use std::sync::Mutex;

static LOG: OnceLock<Mutex<std::fs::File>> = OnceLock::new();

fn init_log() {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("backend.log")
        .expect("Failed to open backend.log");
    LOG.set(Mutex::new(file)).ok();
    log("============================================================");
    log("DLL loaded");
}

fn log(msg: &str) {
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // ts a formato legible: no tenemos chrono, usamos segundos epoch igual
    if let Some(f) = LOG.get() {
        if let Ok(mut f) = f.lock() {
            writeln!(f, "[{}] {}", ts, msg).ok();
        }
    }
}

// ── Version DLL loader ────────────────────────────────────────────────────────

fn load_real_version() {
    unsafe {
        log("Loading real version.dll from System32...");
        let lib = LoadLibraryA(b"C:\\Windows\\System32\\version.dll\0".as_ptr() as _);
        if lib.is_null() {
            log("ERROR: Failed to load System32\\version.dll");
            return;
        }

        macro_rules! get_fn {
            ($name:ident) => {{
                let ptr = GetProcAddress(lib, concat!(stringify!($name), "\0").as_ptr() as _);
                if ptr.is_null() {
                    log(&format!("WARN: GetProcAddress returned null for {}", stringify!($name)));
                }
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

#[no_mangle]
pub extern "system" fn DllMain(_hmodule: *mut u8, reason: u32, _reserved: *mut u8) -> i32 {
    match reason {
        1 => {
            init_log();
            load_real_version();
            init_api_key();
            log("Spawning server thread...");
            thread::spawn(|| run_server());
        }
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
    let path = "key.txt";
    log(&format!("Initializing API key from '{}'...", path));

    if !Path::new(path).exists() {
        log("key.txt not found, creating empty file");
        fs::write(path, "").expect("Failed to create key.txt");
    }

    let key = fs::read_to_string(path)
        .expect("Failed to read key.txt")
        .trim()
        .to_string();

    if key.is_empty() {
        log("WARN: key.txt is empty — API key not set");
    } else {
        log(&format!("API key loaded ({}...)", &key[..key.len().min(8)]));
    }

    API_KEY.set(RwLock::new(key)).ok();
}

fn get_api_key() -> String {
    API_KEY.get().unwrap().read().unwrap().clone()
}

fn set_api_key(new_key: String) {
    log(&format!("Updating API key ({}...)", &new_key[..new_key.len().min(8)]));
    {
        let mut key = API_KEY.get().unwrap().write().unwrap();
        *key = new_key.clone();
    }
    fs::write("key.txt", new_key).expect("Failed to save key");
    log("API key saved to key.txt");
}

// ── Server ────────────────────────────────────────────────────────────────────

const LUA_DIR: &str = "C:\\Program Files (x86)\\Steam\\config\\stplug-in";

fn run_server() {
    log("Starting HTTP server on 127.0.0.1:3000...");
    let server = match Server::http("127.0.0.1:3000") {
        Ok(s) => { log("Server listening on 127.0.0.1:3000"); s }
        Err(e) => { log(&format!("ERROR: Failed to bind server: {}", e)); return; }
    };

    for mut request in server.incoming_requests() {
        let method = request.method().as_str().to_string();
        let url = request.url().to_string();
        log(&format!("--> {} {}", method, url));

        if method != "POST" {
            log(&format!("405 Method Not Allowed ({})", method));
            request.respond(Response::from_string("Method Not Allowed").with_status_code(405)).ok();
            continue;
        }

        // POST /key
        if url.contains("key") {
            log("Handling /key — reading body...");
            let mut body = String::new();
            if request.as_reader().read_to_string(&mut body).is_err() {
                log("ERROR: Failed to read request body for /key");
                request.respond(Response::from_string("Failed to read body").with_status_code(400)).ok();
                continue;
            }
            let key = body.trim();
            log(&format!("Validating key against hubcapmanifest ({}...)", &key[..key.len().min(8)]));

            match ureq::get("https://hubcapmanifest.com/api/v1/user/stats")
                .set("Authorization", &format!("Bearer {}", key))
                .call()
            {
                Ok(_) => {
                    log("Key validation OK");
                    set_api_key(key.to_string());
                    request.respond(Response::from_string("OK").with_status_code(200)).ok();
                    log("<-- 200 OK /key");
                }
                Err(ureq::Error::Status(401, resp)) => {
                    let msg = resp.into_string().unwrap_or_else(|_| "Unauthorized".to_string());
                    log(&format!("Key validation failed 401: {}", msg));
                    request.respond(Response::from_string(msg).with_status_code(401)).ok();
                    log("<-- 401 /key");
                }
                Err(err) => {
                    log(&format!("Key validation error: {}", err));
                    request.respond(Response::from_string(format!("API error: {}", err)).with_status_code(500)).ok();
                    log("<-- 500 /key");
                }
            }
            continue;
        }

        // POST /<appid>
        let appid = url.trim_start_matches('/').to_string();
        log(&format!("Handling appid: {}", appid));

        let mut _body = String::new();
        request.as_reader().read_to_string(&mut _body).ok();

        let fetch_url = format!("https://hubcapmanifest.com/api/v1/lua/{}", appid);
        log(&format!("Fetching: {}", fetch_url));

        let result = ureq::get(&fetch_url)
            .set("Authorization", &format!("Bearer {}", get_api_key()))
            .call();

        match result {
            Ok(resp) => {
                let status = resp.status();
                log(&format!("hubcapmanifest responded {}", status));
                let mut buf = Vec::new();
                resp.into_reader().read_to_end(&mut buf).ok();
                log(&format!("Received {} bytes", buf.len()));

                if let Err(e) = std::fs::create_dir_all(LUA_DIR) {
                    log(&format!("WARN: Failed to create LUA_DIR: {}", e));
                }

                let path = format!("{}\\{}.lua", LUA_DIR, appid);
                match std::fs::write(&path, &buf) {
                    Ok(_) => log(&format!("Saved: {}", path)),
                    Err(e) => log(&format!("ERROR: Failed to write {}: {}", path, e)),
                }

                let response = Response::from_string("OK")
                    .with_header(Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap());
                request.respond(response).ok();
                log("<-- 200 OK");
            }
            Err(ureq::Error::Status(code, resp)) => {
                let msg = resp.into_string().unwrap_or_default();
                log(&format!("ERROR: hubcapmanifest returned {}: {}", code, msg));
                let response = Response::from_string(msg)
                    .with_status_code(code)
                    .with_header(Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap());
                request.respond(response).ok();
                log(&format!("<-- {}", code));
            }
            Err(err) => {
                log(&format!("ERROR: ureq error: {}", err));
                let response = Response::from_string(err.to_string())
                    .with_status_code(500)
                    .with_header(Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap());
                request.respond(response).ok();
                log("<-- 500");
            }
        }
    }
}

// ── Exports ───────────────────────────────────────────────────────────────────

#[no_mangle] pub unsafe extern "system" fn GetFileVersionInfoA(a: *const i8, b: u32, c: u32, d: *mut u8) -> i32 {
    (REAL.get().unwrap().GetFileVersionInfoA)(a, b, c, d)
}
#[no_mangle] pub unsafe extern "system" fn GetFileVersionInfoW(a: *const u16, b: u32, c: u32, d: *mut u8) -> i32 {
    (REAL.get().unwrap().GetFileVersionInfoW)(a, b, c, d)
}
#[no_mangle] pub unsafe extern "system" fn GetFileVersionInfoSizeA(a: *const i8, b: *mut u32) -> u32 {
    (REAL.get().unwrap().GetFileVersionInfoSizeA)(a, b)
}
#[no_mangle] pub unsafe extern "system" fn GetFileVersionInfoSizeW(a: *const u16, b: *mut u32) -> u32 {
    (REAL.get().unwrap().GetFileVersionInfoSizeW)(a, b)
}
#[no_mangle] pub unsafe extern "system" fn GetFileVersionInfoSizeExW(a: u32, b: *const u16, c: *mut u32) -> u32 {
    (REAL.get().unwrap().GetFileVersionInfoSizeExW)(a, b, c)
}
#[no_mangle] pub unsafe extern "system" fn GetFileVersionInfoExW(a: u32, b: *const u16, c: u32, d: u32, e: *mut u8) -> i32 {
    (REAL.get().unwrap().GetFileVersionInfoExW)(a, b, c, d, e)
}
#[no_mangle] pub unsafe extern "system" fn VerFindFileA(a: u32, b: *const i8, c: *const i8, d: *const i8, e: *mut i8, f: *mut u32, g: *mut i8, h: *mut u32) -> u32 {
    (REAL.get().unwrap().VerFindFileA)(a, b, c, d, e, f, g, h)
}
#[no_mangle] pub unsafe extern "system" fn VerFindFileW(a: u32, b: *const u16, c: *const u16, d: *const u16, e: *mut u16, f: *mut u32, g: *mut u16, h: *mut u32) -> u32 {
    (REAL.get().unwrap().VerFindFileW)(a, b, c, d, e, f, g, h)
}
#[no_mangle] pub unsafe extern "system" fn VerInstallFileA(a: u32, b: *const i8, c: *const i8, d: *const i8, e: *const i8, f: *const i8, g: *mut i8, h: *mut u32) -> u32 {
    (REAL.get().unwrap().VerInstallFileA)(a, b, c, d, e, f, g, h)
}
#[no_mangle] pub unsafe extern "system" fn VerInstallFileW(a: u32, b: *const u16, c: *const u16, d: *const u16, e: *const u16, f: *const u16, g: *mut u16, h: *mut u32) -> u32 {
    (REAL.get().unwrap().VerInstallFileW)(a, b, c, d, e, f, g, h)
}
#[no_mangle] pub unsafe extern "system" fn VerLanguageNameA(a: u32, b: *mut i8, c: u32) -> u32 {
    (REAL.get().unwrap().VerLanguageNameA)(a, b, c)
}
#[no_mangle] pub unsafe extern "system" fn VerLanguageNameW(a: u32, b: *mut u16, c: u32) -> u32 {
    (REAL.get().unwrap().VerLanguageNameW)(a, b, c)
}
#[no_mangle] pub unsafe extern "system" fn VerQueryValueA(a: *const u8, b: *const i8, c: *mut *mut u8, d: *mut u32) -> i32 {
    (REAL.get().unwrap().VerQueryValueA)(a, b, c, d)
}
#[no_mangle] pub unsafe extern "system" fn VerQueryValueW(a: *const u8, b: *const u16, c: *mut *mut u8, d: *mut u32) -> i32 {
    (REAL.get().unwrap().VerQueryValueW)(a, b, c, d)
}