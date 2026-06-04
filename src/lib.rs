#![allow(non_snake_case, non_camel_case_types, dead_code, improper_ctypes)]

use std::thread;
use std::sync::OnceLock;
use std::io::Read;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use tiny_http::{Server, Response, Header};
use std::fs;
use std::path::Path;
use std::sync::{RwLock, Mutex};

// ── Version DLL types ─────────────────────────────────────────────────────────

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

// Garantiza que REAL se inicialice una sola vez, incluso si los exports
// se llaman antes de que el thread de init termine.
fn ensure_real() {
    if REAL.get().is_some() { return; }
    unsafe {
        // Obtenemos system32 sin usar SYSTEM32_DIR (puede no estar init todavía)
        let mut sys_buf = vec![0u16; 260];
        let sys_len = GetSystemDirectoryW(sys_buf.as_mut_ptr(), 260);
        let system32 = if sys_len > 0 {
            sys_buf.truncate(sys_len as usize);
            OsString::from_wide(&sys_buf).to_string_lossy().to_string()
        } else {
            "C:\\Windows\\System32".to_string()
        };
        let real_path = format!("{}\\version.dll\0", system32);
        let lib = LoadLibraryA(real_path.as_ptr() as _);
        if lib.is_null() { return; }
        macro_rules! gfn {
            ($name:ident) => {{
                let ptr = GetProcAddress(lib, concat!(stringify!($name), "\0").as_ptr() as _);
                std::mem::transmute(ptr)
            }};
        }
        REAL.set(VersionFns {
            GetFileVersionInfoA:    gfn!(GetFileVersionInfoA),
            GetFileVersionInfoW:    gfn!(GetFileVersionInfoW),
            GetFileVersionInfoSizeA: gfn!(GetFileVersionInfoSizeA),
            GetFileVersionInfoSizeW: gfn!(GetFileVersionInfoSizeW),
            GetFileVersionInfoSizeExW: gfn!(GetFileVersionInfoSizeExW),
            GetFileVersionInfoExW:  gfn!(GetFileVersionInfoExW),
            VerFindFileA:           gfn!(VerFindFileA),
            VerFindFileW:           gfn!(VerFindFileW),
            VerInstallFileA:        gfn!(VerInstallFileA),
            VerInstallFileW:        gfn!(VerInstallFileW),
            VerLanguageNameA:       gfn!(VerLanguageNameA),
            VerLanguageNameW:       gfn!(VerLanguageNameW),
            VerQueryValueA:         gfn!(VerQueryValueA),
            VerQueryValueW:         gfn!(VerQueryValueW),
        }).ok();
    }
}

// ── Win32 imports ─────────────────────────────────────────────────────────────

extern "system" {
    fn LoadLibraryA(name: *const i8) -> *mut u8;
    fn GetProcAddress(module: *mut u8, name: *const i8) -> *mut u8;
    fn GetModuleFileNameW(hmodule: *mut u8, filename: *mut u16, size: u32) -> u32;
    fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
    fn GetModuleHandleA(name: *const i8) -> *mut u8;
    fn VirtualProtect(addr: *mut u8, size: usize, new_protect: u32, old_protect: *mut u32) -> i32;
}

const PAGE_EXECUTE_READWRITE: u32 = 0x40;

// ── CEF C API types ───────────────────────────────────────────────────────────

#[repr(C)]
struct CefStringUtf16 {
    str_: *mut u16,
    length: usize,
    dtor: Option<unsafe extern "C" fn(*mut u16)>,
}

impl CefStringUtf16 {
    fn from_str(s: &str) -> Self {
        let wide: Vec<u16> = s.encode_utf16().collect();
        let len = wide.len();
        let ptr = {
            let mut boxed = wide.into_boxed_slice();
            let p = boxed.as_mut_ptr();
            std::mem::forget(boxed);
            p
        };
        CefStringUtf16 { str_: ptr, length: len, dtor: None }
    }

    unsafe fn to_string(&self) -> String {
        if self.str_.is_null() || self.length == 0 { return String::new(); }
        let slice = std::slice::from_raw_parts(self.str_, self.length);
        String::from_utf16_lossy(slice)
    }
}

#[repr(C)]
struct CefBaseRefCounted {
    size: usize,
    add_ref: Option<unsafe extern "C" fn(*mut CefBaseRefCounted)>,
    release: Option<unsafe extern "C" fn(*mut CefBaseRefCounted) -> i32>,
    has_one_ref: Option<unsafe extern "C" fn(*mut CefBaseRefCounted) -> i32>,
    has_at_least_one_ref: Option<unsafe extern "C" fn(*mut CefBaseRefCounted) -> i32>,
}

#[repr(C)]
struct CefFrame {
    base: CefBaseRefCounted,
    is_valid: *mut u8,
    undo: *mut u8,
    redo: *mut u8,
    cut: *mut u8,
    copy: *mut u8,
    paste: *mut u8,
    del: *mut u8,
    select_all: *mut u8,
    view_source: *mut u8,
    get_source: *mut u8,
    get_text: *mut u8,
    load_request: *mut u8,
    load_url: *mut u8,
    execute_java_script: Option<
        unsafe extern "C" fn(*mut CefFrame, *const CefStringUtf16, *const CefStringUtf16, i32)
    >,
    is_main: Option<unsafe extern "C" fn(*mut CefFrame) -> i32>,
    is_focused: *mut u8,
    get_name: *mut u8,
    get_identifier: *mut u8,
    get_parent: *mut u8,
    get_url: Option<unsafe extern "C" fn(*mut CefFrame) -> *mut CefStringUtf16>,
}

type FnOnLoadEnd = unsafe extern "C" fn(*mut u8, *mut u8, *mut CefFrame, i32);
type FnGetLoadHandler = unsafe extern "C" fn(*mut u8) -> *mut u8;
type FnCreateBrowser = unsafe extern "C" fn(*mut u8, *mut u8, *const CefStringUtf16, *mut u8, *mut u8, *mut u8) -> i32;

static mut ORIG_CREATE_BROWSER: Option<FnCreateBrowser> = None;
static mut ORIG_GET_LOAD_HANDLER: Option<FnGetLoadHandler> = None;
static mut ORIG_ON_LOAD_END: Option<FnOnLoadEnd> = None;

const ON_LOAD_END_VTABLE_IDX: usize = 7;
const GET_LOAD_HANDLER_VTABLE_IDX: usize = 15;

// ── Hooks ─────────────────────────────────────────────────────────────────────

unsafe extern "C" fn hooked_on_load_end(
    handler: *mut u8,
    browser: *mut u8,
    frame: *mut CefFrame,
    http_status_code: i32,
) {
    if let Some(orig) = ORIG_ON_LOAD_END {
        orig(handler, browser, frame, http_status_code);
    }
    if frame.is_null() { return; }
    if let Some(is_main) = (*frame).is_main {
        if is_main(frame) == 0 { return; }
    }
    let url = if let Some(get_url) = (*frame).get_url {
        let cef_str = get_url(frame);
        if cef_str.is_null() { return; }
        (*cef_str).to_string()
    } else { return; };

    if !url.starts_with("https://store.steampowered.com/app/") { return; }

    log(&format!("Inyectando JS en: {}", url));
    let js_path = format!("{}\\content.js", steam_dir());
    let js_code = match std::fs::read_to_string(&js_path) {
        Ok(code) => code,
        Err(e) => { log(&format!("ERROR leyendo content.js: {}", e)); return; }
    };
    if let Some(exec) = (*frame).execute_java_script {
        let code_str = CefStringUtf16::from_str(&js_code);
        let url_str  = CefStringUtf16::from_str("content.js");
        exec(frame, &code_str, &url_str, 0);
        log("JS inyectado OK");
    }
}

unsafe extern "C" fn hooked_get_load_handler(client: *mut u8) -> *mut u8 {
    let handler = if let Some(orig) = ORIG_GET_LOAD_HANDLER {
        orig(client)
    } else { return std::ptr::null_mut(); };
    if handler.is_null() { return handler; }
    patch_vtable(handler, ON_LOAD_END_VTABLE_IDX, hooked_on_load_end as *mut u8, |orig_ptr| {
        ORIG_ON_LOAD_END = Some(std::mem::transmute(orig_ptr));
    });
    handler
}

unsafe extern "C" fn hooked_create_browser(
    window_info: *mut u8,
    client: *mut u8,
    url: *const CefStringUtf16,
    settings: *mut u8,
    extra_info: *mut u8,
    request_context: *mut u8,
) -> i32 {
    if !client.is_null() {
        patch_vtable(client, GET_LOAD_HANDLER_VTABLE_IDX, hooked_get_load_handler as *mut u8, |orig_ptr| {
            ORIG_GET_LOAD_HANDLER = Some(std::mem::transmute(orig_ptr));
        });
    }
    if let Some(orig) = ORIG_CREATE_BROWSER {
        orig(window_info, client, url, settings, extra_info, request_context)
    } else { 0 }
}

// ── Patch helpers ─────────────────────────────────────────────────────────────

unsafe fn patch_vtable<F>(obj: *mut u8, slot_idx: usize, new_fn: *mut u8, save_orig: F)
where F: FnOnce(*mut u8) {
    let vtable = *(obj as *mut *mut *mut u8);
    let slot = vtable.add(slot_idx);
    let orig = *slot;
    save_orig(orig);
    let mut old: u32 = 0;
    VirtualProtect(slot as *mut u8, 8, PAGE_EXECUTE_READWRITE, &mut old);
    *slot = new_fn;
    VirtualProtect(slot as *mut u8, 8, old, &mut old);
}

unsafe fn patch_fn_jmp(target: *mut u8, new_fn: *mut u8) {
    let mut old: u32 = 0;
    VirtualProtect(target, 14, PAGE_EXECUTE_READWRITE, &mut old);
    let p = std::slice::from_raw_parts_mut(target, 14);
    p[0] = 0xFF; p[1] = 0x25;
    p[2] = 0x00; p[3] = 0x00; p[4] = 0x00; p[5] = 0x00;
    p[6..14].copy_from_slice(&(new_fn as u64).to_le_bytes());
    VirtualProtect(target, 14, old, &mut old);
}

// ── CEF hook ──────────────────────────────────────────────────────────────────

fn start_cef_hook() {
    thread::spawn(|| {
        log("Esperando libcef.dll...");
        let libcef = loop {
            let h = unsafe { GetModuleHandleA(b"libcef.dll\0".as_ptr() as _) };
            if !h.is_null() { break h; }
            thread::sleep(std::time::Duration::from_millis(100));
        };
        log("libcef.dll encontrada");
        unsafe {
            let create_browser = GetProcAddress(libcef, b"cef_browser_host_create_browser\0".as_ptr() as _);
            if create_browser.is_null() {
                log("ERROR: cef_browser_host_create_browser no encontrado");
                return;
            }
            log(&format!("cef_browser_host_create_browser @ {:p}", create_browser));
            ORIG_CREATE_BROWSER = Some(std::mem::transmute(create_browser));
            patch_fn_jmp(create_browser, hooked_create_browser as *mut u8);
            log("Hook instalado OK");
        }
    });
}

// ── Paths ─────────────────────────────────────────────────────────────────────

static STEAM_DIR: OnceLock<String> = OnceLock::new();
static SYSTEM32_DIR: OnceLock<String> = OnceLock::new();

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
            let dll_dir = full.rfind('\\')
                .map(|i| full[..i].to_string())
                .unwrap_or_else(|| ".".to_string());
            let steam_dir = if dll_dir.to_lowercase().contains("cef.win64") {
                Path::new(&dll_dir)
                    .ancestors()
                    .nth(3)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or(dll_dir)
            } else {
                dll_dir
            };
            STEAM_DIR.set(steam_dir).ok();
        } else {
            STEAM_DIR.set(
                std::env::current_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|_| ".".to_string())
            ).ok();
        }
    }
}

fn steam_dir() -> &'static str { STEAM_DIR.get().map(|s| s.as_str()).unwrap_or(".") }
fn system32_dir() -> &'static str { SYSTEM32_DIR.get().map(|s| s.as_str()).unwrap_or("C:\\Windows\\System32") }
fn lua_dir() -> String { format!("{}\\config\\stplug-in", steam_dir()) }
fn key_path() -> String { format!("{}\\key.txt", steam_dir()) }
fn log_path() -> String { format!("{}\\backend.log", steam_dir()) }

// ── Logger ────────────────────────────────────────────────────────────────────

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

// ── Version DLL loader ────────────────────────────────────────────────────────

fn load_real_version() {
    unsafe {
        let path = format!("{}\\version.dll", system32_dir());
        log(&format!("Loading real version.dll from {}...", path));
        let mut path_bytes: Vec<u8> = path.bytes().collect();
        path_bytes.push(0);
        let lib = LoadLibraryA(path_bytes.as_ptr() as _);
        if lib.is_null() { log("ERROR: Failed to load real version.dll"); return; }
        macro_rules! get_fn {
            ($name:ident) => {{
                let ptr = GetProcAddress(lib, concat!(stringify!($name), "\0").as_ptr() as _);
                if ptr.is_null() { log(&format!("WARN: null for {}", stringify!($name))); }
                std::mem::transmute(ptr)
            }};
        }
        REAL.set(VersionFns {
            GetFileVersionInfoA:      get_fn!(GetFileVersionInfoA),
            GetFileVersionInfoW:      get_fn!(GetFileVersionInfoW),
            GetFileVersionInfoSizeA:  get_fn!(GetFileVersionInfoSizeA),
            GetFileVersionInfoSizeW:  get_fn!(GetFileVersionInfoSizeW),
            GetFileVersionInfoSizeExW:get_fn!(GetFileVersionInfoSizeExW),
            GetFileVersionInfoExW:    get_fn!(GetFileVersionInfoExW),
            VerFindFileA:             get_fn!(VerFindFileA),
            VerFindFileW:             get_fn!(VerFindFileW),
            VerInstallFileA:          get_fn!(VerInstallFileA),
            VerInstallFileW:          get_fn!(VerInstallFileW),
            VerLanguageNameA:         get_fn!(VerLanguageNameA),
            VerLanguageNameW:         get_fn!(VerLanguageNameW),
            VerQueryValueA:           get_fn!(VerQueryValueA),
            VerQueryValueW:           get_fn!(VerQueryValueW),
        }).ok();
        log("version.dll exports loaded OK");
    }
}

// ── API Key ───────────────────────────────────────────────────────────────────

static API_KEY: OnceLock<RwLock<String>> = OnceLock::new();

fn init_api_key() {
    let path = key_path();
    if !Path::new(&path).exists() { fs::write(&path, "").ok(); }
    let key = fs::read_to_string(&path).unwrap_or_default().trim().to_string();
    if key.is_empty() { log("WARN: key.txt is empty"); }
    else { log(&format!("API key loaded ({}...)", &key[..key.len().min(8)])); }
    API_KEY.set(RwLock::new(key)).ok();
}

fn get_api_key() -> String { API_KEY.get().unwrap().read().unwrap().clone() }

fn set_api_key(new_key: String) {
    { *API_KEY.get().unwrap().write().unwrap() = new_key.clone(); }
    fs::write(key_path(), new_key).ok();
    log("API key saved");
}

fn cors_header() -> Header {
    Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap()
}

fn is_primary_process() -> bool {
    std::net::TcpStream::connect("127.0.0.1:3000").is_err()
}

// ── DllMain ───────────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "system" fn DllMain(hmodule: *mut u8, reason: u32, _reserved: *mut u8) -> i32 {
    if reason == 1 {
        // Todo diferido — no hacemos NADA en el loader lock
        thread::spawn(move || {
            // Pequeño sleep para que el proceso termine de inicializar
            thread::sleep(std::time::Duration::from_millis(500));
            init_paths(hmodule);
            init_log();
            load_real_version();
            init_api_key();
            start_cef_hook();

            if is_primary_process() {
                log("Primary process — starting server");
                run_server();
            } else {
                log("Secondary process — skipping server");
            }
        });
    }
    1
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
        let path = url.trim_start_matches('/').split('?').next().unwrap_or("").to_string();
        log(&format!("--> {} {}", method, url));

        if method == "OPTIONS" {
            let response = Response::from_string("")
                .with_header(cors_header())
                .with_header(Header::from_bytes(&b"Access-Control-Allow-Methods"[..], &b"GET, POST, OPTIONS"[..]).unwrap())
                .with_header(Header::from_bytes(&b"Access-Control-Allow-Headers"[..], &b"*"[..]).unwrap());
            request.respond(response).ok();
            continue;
        }
        if method != "POST" {
            request.respond(Response::from_string("Method Not Allowed").with_status_code(405)).ok();
            continue;
        }

        if path == "key" {
            let mut body = String::new();
            if request.as_reader().read_to_string(&mut body).is_err() {
                request.respond(Response::from_string("Bad Request").with_status_code(400)).ok();
                continue;
            }
            let key = body.trim().to_string();
            match ureq::get("https://hubcapmanifest.com/api/v1/user/stats")
                .set("Authorization", &format!("Bearer {}", key))
                .call()
            {
                Ok(_) => {
                    set_api_key(key);
                    request.respond(Response::from_string("OK").with_status_code(200).with_header(cors_header())).ok();
                }
                Err(ureq::Error::Status(401, resp)) => {
                    let msg = resp.into_string().unwrap_or_else(|_| "Unauthorized".to_string());
                    request.respond(Response::from_string(msg).with_status_code(401).with_header(cors_header())).ok();
                }
                Err(err) => {
                    request.respond(Response::from_string(err.to_string()).with_status_code(500).with_header(cors_header())).ok();
                }
            }
            continue;
        }

        let appid = path.clone();
        let mut _body = String::new();
        request.as_reader().read_to_string(&mut _body).ok();
        let fetch_url = format!("https://hubcapmanifest.com/api/v1/lua/{}", appid);

        match ureq::get(&fetch_url)
            .set("Authorization", &format!("Bearer {}", get_api_key()))
            .call()
        {
            Ok(resp) => {
                let mut buf = Vec::new();
                resp.into_reader().read_to_end(&mut buf).ok();
                std::fs::create_dir_all(lua_dir()).ok();
                let out_path = format!("{}\\{}.lua", lua_dir(), appid);
                std::fs::write(&out_path, &buf).ok();
                log(&format!("Saved: {}", out_path));
                request.respond(Response::from_string("OK").with_header(cors_header())).ok();
            }
            Err(ureq::Error::Status(code, resp)) => {
                let msg = resp.into_string().unwrap_or_default();
                request.respond(Response::from_string(msg).with_status_code(code).with_header(cors_header())).ok();
            }
            Err(err) => {
                request.respond(Response::from_string(err.to_string()).with_status_code(500).with_header(cors_header())).ok();
            }
        }
    }
}

// ── Version DLL exports ───────────────────────────────────────────────────────

#[no_mangle] pub unsafe extern "system" fn GetFileVersionInfoA(a: *const i8, b: u32, c: u32, d: *mut u8) -> i32 { ensure_real(); (REAL.get().unwrap().GetFileVersionInfoA)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn GetFileVersionInfoW(a: *const u16, b: u32, c: u32, d: *mut u8) -> i32 { ensure_real(); (REAL.get().unwrap().GetFileVersionInfoW)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn GetFileVersionInfoSizeA(a: *const i8, b: *mut u32) -> u32 { ensure_real(); (REAL.get().unwrap().GetFileVersionInfoSizeA)(a, b) }
#[no_mangle] pub unsafe extern "system" fn GetFileVersionInfoSizeW(a: *const u16, b: *mut u32) -> u32 { ensure_real(); (REAL.get().unwrap().GetFileVersionInfoSizeW)(a, b) }
#[no_mangle] pub unsafe extern "system" fn GetFileVersionInfoSizeExW(a: u32, b: *const u16, c: *mut u32) -> u32 { ensure_real(); (REAL.get().unwrap().GetFileVersionInfoSizeExW)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn GetFileVersionInfoExW(a: u32, b: *const u16, c: u32, d: u32, e: *mut u8) -> i32 { ensure_real(); (REAL.get().unwrap().GetFileVersionInfoExW)(a, b, c, d, e) }
#[no_mangle] pub unsafe extern "system" fn VerFindFileA(a: u32, b: *const i8, c: *const i8, d: *const i8, e: *mut i8, f: *mut u32, g: *mut i8, h: *mut u32) -> u32 { ensure_real(); (REAL.get().unwrap().VerFindFileA)(a, b, c, d, e, f, g, h) }
#[no_mangle] pub unsafe extern "system" fn VerFindFileW(a: u32, b: *const u16, c: *const u16, d: *const u16, e: *mut u16, f: *mut u32, g: *mut u16, h: *mut u32) -> u32 { ensure_real(); (REAL.get().unwrap().VerFindFileW)(a, b, c, d, e, f, g, h) }
#[no_mangle] pub unsafe extern "system" fn VerInstallFileA(a: u32, b: *const i8, c: *const i8, d: *const i8, e: *const i8, f: *const i8, g: *mut i8, h: *mut u32) -> u32 { ensure_real(); (REAL.get().unwrap().VerInstallFileA)(a, b, c, d, e, f, g, h) }
#[no_mangle] pub unsafe extern "system" fn VerInstallFileW(a: u32, b: *const u16, c: *const u16, d: *const u16, e: *const u16, f: *const u16, g: *mut u16, h: *mut u32) -> u32 { ensure_real(); (REAL.get().unwrap().VerInstallFileW)(a, b, c, d, e, f, g, h) }
#[no_mangle] pub unsafe extern "system" fn VerLanguageNameA(a: u32, b: *mut i8, c: u32) -> u32 { ensure_real(); (REAL.get().unwrap().VerLanguageNameA)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn VerLanguageNameW(a: u32, b: *mut u16, c: u32) -> u32 { ensure_real(); (REAL.get().unwrap().VerLanguageNameW)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn VerQueryValueA(a: *const u8, b: *const i8, c: *mut *mut u8, d: *mut u32) -> i32 { ensure_real(); (REAL.get().unwrap().VerQueryValueA)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn VerQueryValueW(a: *const u8, b: *const u16, c: *mut *mut u8, d: *mut u32) -> i32 { ensure_real(); (REAL.get().unwrap().VerQueryValueW)(a, b, c, d) }