#![allow(non_snake_case, non_camel_case_types, dead_code, improper_ctypes)]

use std::thread;
use std::sync::OnceLock;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::io::Read;

// ─── Configurable ──────────────────────────────────────────────────────────────

const GITHUB_REPO: &str = "Peron4TheWin/steampluginbackpy";
const BACKEND_EXE: &str = "backend.exe";

// ── Win32 imports ─────────────────────────────────────────────────────────────

extern "system" {
    fn GetCommandLineW() -> *const u16;
    fn GetModuleFileNameW(hmodule: *mut u8, filename: *mut u16, size: u32) -> u32;
    fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
    fn CreateProcessW(
        app: *const u16,
        cmd: *mut u16,
        pa: *mut u8,
        ta: *mut u8,
        inherit: i32,
        flags: u32,
        env: *mut u8,
        dir: *const u16,
        si: *mut u8,
        pi: *mut u8,
    ) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteW(
        hwnd: *mut u8,
        operation: *const u16,
        file: *const u16,
        params: *const u16,
        dir: *const u16,
        show: i32,
    ) -> isize;
}

// DETACHED_PROCESS | CREATE_NO_WINDOW
const CREATE_NO_WINDOW: u32 = 0x08000000;
const DETACHED_PROCESS: u32 = 0x00000008;

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
            "C:\\Windows\\System32".to_string()
        };
        SYSTEM32_DIR.set(system32).ok();

        let mut dll_buf = vec![0u16; 260];
        let dll_len = GetModuleFileNameW(hmodule, dll_buf.as_mut_ptr(), 260);
        let dir = if dll_len > 0 {
            dll_buf.truncate(dll_len as usize);
            let full = OsString::from_wide(&dll_buf).to_string_lossy().to_string();
            full.rfind('\\')
                .map(|i| full[..i].to_string())
                .unwrap_or_else(|| ".".to_string())
        } else {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string())
        };
        STEAM_DIR.set(dir).ok();
    }
}

fn steam_dir() -> &'static str { STEAM_DIR.get().map(|s| s.as_str()).unwrap_or(".") }
fn log_path() -> String { format!("{}\\version_dll.log", steam_dir()) }
fn backend_path() -> String { format!("{}\\{}", steam_dir(), BACKEND_EXE) }
fn cef_bat_path() -> String { format!("{}\\cef.bat", steam_dir()) }
fn cef_stamp_path() -> String { format!("{}\\cef.stamp", steam_dir()) }

// ── Logger ────────────────────────────────────────────────────────────────────

static LOG: OnceLock<Mutex<std::fs::File>> = OnceLock::new();

fn init_log() {
    let file = std::fs::OpenOptions::new()
        .create(true).append(true)
        .open(log_path())
        .expect("Failed to open log");
    LOG.set(Mutex::new(file)).ok();
    log("============================================================");
    log("DLL loaded");
    log(&format!("Steam dir: {}", steam_dir()));
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

// ── Version DLL proxy ─────────────────────────────────────────────────────────

type FnGetFileVersionInfoA     = unsafe extern "system" fn(*const i8, u32, u32, *mut u8) -> i32;
type FnGetFileVersionInfoW     = unsafe extern "system" fn(*const u16, u32, u32, *mut u8) -> i32;
type FnGetFileVersionInfoSizeA = unsafe extern "system" fn(*const i8, *mut u32) -> u32;
type FnGetFileVersionInfoSizeW = unsafe extern "system" fn(*const u16, *mut u32) -> u32;
type FnGetFileVersionInfoSizeExW = unsafe extern "system" fn(u32, *const u16, *mut u32) -> u32;
type FnGetFileVersionInfoExW   = unsafe extern "system" fn(u32, *const u16, u32, u32, *mut u8) -> i32;
type FnVerFindFileA   = unsafe extern "system" fn(u32, *const i8, *const i8, *const i8, *mut i8, *mut u32, *mut i8, *mut u32) -> u32;
type FnVerFindFileW   = unsafe extern "system" fn(u32, *const u16, *const u16, *const u16, *mut u16, *mut u32, *mut u16, *mut u32) -> u32;
type FnVerInstallFileA = unsafe extern "system" fn(u32, *const i8, *const i8, *const i8, *const i8, *const i8, *mut i8, *mut u32) -> u32;
type FnVerInstallFileW = unsafe extern "system" fn(u32, *const u16, *const u16, *const u16, *const u16, *const u16, *mut u16, *mut u32) -> u32;
type FnVerLanguageNameA = unsafe extern "system" fn(u32, *mut i8, u32) -> u32;
type FnVerLanguageNameW = unsafe extern "system" fn(u32, *mut u16, u32) -> u32;
type FnVerQueryValueA = unsafe extern "system" fn(*const u8, *const i8, *mut *mut u8, *mut u32) -> i32;
type FnVerQueryValueW = unsafe extern "system" fn(*const u8, *const u16, *mut *mut u8, *mut u32) -> i32;

struct VersionFns {
    GetFileVersionInfoA:      FnGetFileVersionInfoA,
    GetFileVersionInfoW:      FnGetFileVersionInfoW,
    GetFileVersionInfoSizeA:  FnGetFileVersionInfoSizeA,
    GetFileVersionInfoSizeW:  FnGetFileVersionInfoSizeW,
    GetFileVersionInfoSizeExW:FnGetFileVersionInfoSizeExW,
    GetFileVersionInfoExW:    FnGetFileVersionInfoExW,
    VerFindFileA:             FnVerFindFileA,
    VerFindFileW:             FnVerFindFileW,
    VerInstallFileA:          FnVerInstallFileA,
    VerInstallFileW:          FnVerInstallFileW,
    VerLanguageNameA:         FnVerLanguageNameA,
    VerLanguageNameW:         FnVerLanguageNameW,
    VerQueryValueA:           FnVerQueryValueA,
    VerQueryValueW:           FnVerQueryValueW,
}

unsafe impl Send for VersionFns {}
unsafe impl Sync for VersionFns {}

static REAL: OnceLock<VersionFns> = OnceLock::new();

extern "system" {
    fn LoadLibraryA(name: *const i8) -> *mut u8;
    fn GetProcAddress(module: *mut u8, name: *const i8) -> *mut u8;
}

fn load_real_version() {
    unsafe {
        let path = format!("{}\\version.dll\0", SYSTEM32_DIR.get().unwrap());
        let lib = LoadLibraryA(path.as_ptr() as _);
        if lib.is_null() { log("ERROR: no se pudo cargar version.dll real"); return; }
        macro_rules! gfn {
            ($name:ident) => {{
                let ptr = GetProcAddress(lib, concat!(stringify!($name), "\0").as_ptr() as _);
                std::mem::transmute(ptr)
            }};
        }
        REAL.set(VersionFns {
            GetFileVersionInfoA:       gfn!(GetFileVersionInfoA),
            GetFileVersionInfoW:       gfn!(GetFileVersionInfoW),
            GetFileVersionInfoSizeA:   gfn!(GetFileVersionInfoSizeA),
            GetFileVersionInfoSizeW:   gfn!(GetFileVersionInfoSizeW),
            GetFileVersionInfoSizeExW: gfn!(GetFileVersionInfoSizeExW),
            GetFileVersionInfoExW:     gfn!(GetFileVersionInfoExW),
            VerFindFileA:              gfn!(VerFindFileA),
            VerFindFileW:              gfn!(VerFindFileW),
            VerInstallFileA:           gfn!(VerInstallFileA),
            VerInstallFileW:           gfn!(VerInstallFileW),
            VerLanguageNameA:          gfn!(VerLanguageNameA),
            VerLanguageNameW:          gfn!(VerLanguageNameW),
            VerQueryValueA:            gfn!(VerQueryValueA),
            VerQueryValueW:            gfn!(VerQueryValueW),
        }).ok();
        log("version.dll real cargado OK");
    }
}

// ── CEF debug check ───────────────────────────────────────────────────────────

fn has_cef_debug_flag() -> bool {
    unsafe {
        let ptr = GetCommandLineW();
        if ptr.is_null() { return false; }
        let mut len = 0usize;
        while *ptr.add(len) != 0 { len += 1; }
        let cmdline = OsString::from_wide(std::slice::from_raw_parts(ptr, len))
            .to_string_lossy()
            .to_lowercase();
        cmdline.contains("-cef-enable-debugging")
    }
}

fn launch_cef_bat() {
    let bat = cef_bat_path();
    if !Path::new(&bat).exists() {
        log("cef.bat no encontrado, creandolo...");
        let content = "@echo off\r\ntaskkill /im steam.exe /f\r\nstart \"\" steam.exe -cef-enable-debugging\r\nexit\r\n";
        match fs::write(&bat, content) {
            Ok(_)  => log("cef.bat creado OK"),
            Err(e) => { log(&format!("ERROR creando cef.bat: {}", e)); return; }
        }
    }

    // Escribimos el timestamp ANTES de lanzar el bat
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    fs::write(cef_stamp_path(), now.to_string()).ok();
    log(&format!("Stamp escrito (ts={}), lanzando cef.bat...", now));

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe {
        let op   = to_wide("open");
        let file = to_wide(&bat);
        let dir  = to_wide(steam_dir());

        let result = ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            dir.as_ptr(),
            0, // SW_HIDE
        );
        if result > 32 {
            log("cef.bat lanzado OK");
        } else {
            // Si falló lanzar, reseteamos el stamp para que lo reintente
            fs::write(cef_stamp_path(), "0").ok();
            log(&format!("ERROR lanzando cef.bat: ShellExecuteW devolvio {}", result));
        }
    }
}

// ── SHA256 ────────────────────────────────────────────────────────────────────

fn sha256_file(path: &str) -> Option<String> {
    use std::io::BufReader;

    // SHA-256 manual (sin deps externos) usando la crate sha2 que ya debe estar en Cargo.toml
    // Si no tenés sha2, agregá: sha2 = "0.10"
    // Acá usamos sha2::Digest
    use sha2::{Sha256, Digest};

    let file = fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = reader.read(&mut buf).ok()?;
        if n == 0 { break; }
        hasher.update(&buf[..n]);
    }
    let result = hasher.finalize();
    Some(format!("{:x}", result))
}

// ── GitHub release check + download ──────────────────────────────────────────

// Parseo mínimo del JSON de GitHub sin serde
// Extrae el valor string de la primera ocurrencia de "key": "value" en el slice dado
fn extract_str_after<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{}\":", key);
    let start = json.find(&needle)? + needle.len();
    let rest = json[start..].trim_start();
    if !rest.starts_with('"') { return None; }
    let inner = &rest[1..];
    let end = inner.find('"')?;
    Some(&inner[..end])
}

// Busca en el JSON del release el asset .exe y devuelve (browser_download_url, sha256)
// El SHA256 viene en el campo "digest": "sha256:abc123..."
// https://docs.github.com/en/rest/releases/assets
fn find_exe_asset(json: &str) -> Option<(String, String)> {
    let mut search = json;
    while let Some(pos) = search.find("\"name\":") {
        let rest = &search[pos..];
        if let Some(name) = extract_str_after(rest, "name") {
            if name.ends_with(".exe") {
                let url = extract_str_after(rest, "browser_download_url")
                    .unwrap_or("")
                    .to_string();
                // "digest": "sha256:abc123..."
                let sha = extract_str_after(rest, "digest")
                    .and_then(|d| d.strip_prefix("sha256:"))
                    .unwrap_or("")
                    .to_lowercase();
                return Some((url, sha));
            }
        }
        search = &search[pos + 7..];
    }
    None
}

fn check_and_update_backend() {
    let api_url = format!("https://api.github.com/repos/{}/releases/latest", GITHUB_REPO);
    log(&format!("Consultando GitHub: {}", api_url));

    let release_json = match ureq::get(&api_url)
        .set("User-Agent", "version-dll/1.0")
        .set("Accept", "application/vnd.github+json")
        .call()
    {
        Ok(resp) => {
            let mut s = String::new();
            resp.into_reader().read_to_string(&mut s).unwrap_or(0);
            s
        }
        Err(e) => {
            log(&format!("ERROR consultando GitHub API: {}", e));
            return;
        }
    };

    let (exe_url, expected_sha) = match find_exe_asset(&release_json) {
        Some(v) => v,
        None => {
            log("ERROR: no se encontro asset .exe en el latest release");
            return;
        }
    };
    log(&format!("Asset: {} | SHA256 esperado: {}", exe_url, expected_sha));

    if expected_sha.is_empty() {
        log("WARN: el asset no tiene campo digest, no se puede verificar SHA256");
        return;
    }

    // SHA256 del backend.exe local
    let local_path = backend_path();
    let local_sha = if Path::new(&local_path).exists() {
        sha256_file(&local_path).unwrap_or_default()
    } else {
        String::new()
    };
    log(&format!("SHA256 local:    {}", local_sha));

    if local_sha == expected_sha {
        log("backend.exe esta actualizado, nada que hacer");
    } else {
        log("SHA256 no coincide, descargando nueva version...");

        // Borramos el viejo
        if Path::new(&local_path).exists() {
            if let Err(e) = fs::remove_file(&local_path) {
                log(&format!("ERROR borrando backend.exe viejo: {}", e));
                return;
            }
            log("backend.exe viejo borrado");
        }

        // Descargamos
        match ureq::get(&exe_url)
            .set("User-Agent", "version-dll/1.0")
            .call()
        {
            Ok(resp) => {
                let mut buf = Vec::new();
                resp.into_reader().read_to_end(&mut buf).ok();
                match fs::write(&local_path, &buf) {
                    Ok(_) => log(&format!("backend.exe descargado ({} bytes)", buf.len())),
                    Err(e) => {
                        log(&format!("ERROR escribiendo backend.exe: {}", e));
                        return;
                    }
                }
                // Verificamos el SHA del recien bajado
                let new_sha = sha256_file(&local_path).unwrap_or_default();
                if new_sha == expected_sha {
                    log("SHA256 verificado OK post-descarga");
                } else {
                    log(&format!("WARN: SHA256 post-descarga no coincide! ({}) - puede estar corrupto", new_sha));
                }
            }
            Err(e) => {
                log(&format!("ERROR descargando backend.exe: {}", e));
                return;
            }
        }
    }

    // Lanzamos backend.exe en background
    launch_backend();
}

fn launch_backend() {
    let path = backend_path();
    if !Path::new(&path).exists() {
        log("backend.exe no existe, no se puede lanzar");
        return;
    }

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    // STARTUPINFOW es 104 bytes en x64; lo inicializamos con ceros
    let mut si = vec![0u8; 104];
    // cb = sizeof(STARTUPINFOW) = 104
    si[0] = 104;
    // PROCESS_INFORMATION = 24 bytes
    let mut pi = vec![0u8; 24];

    let app = to_wide(&path);
    let mut cmd = to_wide(&format!("\"{}\"", path));
    let dir = to_wide(steam_dir());

    unsafe {
        let result = CreateProcessW(
            app.as_ptr(),
            cmd.as_mut_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            DETACHED_PROCESS | CREATE_NO_WINDOW,
            std::ptr::null_mut(),
            dir.as_ptr(),
            si.as_mut_ptr(),
            pi.as_mut_ptr(),
        );
        if result != 0 {
            log("backend.exe lanzado en background OK");
        } else {
            log("ERROR: CreateProcessW fallo al lanzar backend.exe");
        }
    }
}

// ── DllMain ───────────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "system" fn DllMain(hmodule: *mut u8, reason: u32, _reserved: *mut u8) -> i32 {
    if reason == 1 {
        let hmodule_addr = hmodule as usize;
        thread::spawn(move || {
            init_paths(hmodule_addr as *mut u8);
            init_log();
            load_real_version();

            // 1. Chequear CEF debug flag
            if has_cef_debug_flag() {
                log("CEF debugging habilitado OK");
            } else {
                // Chequeamos si lanzamos el bat hace menos de 10 segundos
                // (ventana de tiempo para que Steam se mate y se reinicie)
                let stamp_path = cef_stamp_path();
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                let last_launch = fs::read_to_string(&stamp_path)
                    .ok()
                    .and_then(|s| s.trim().parse::<u64>().ok())
                    .unwrap_or(0);

                if now.saturating_sub(last_launch) < 10 {
                    log(&format!("Bat lanzado hace {}s, esperando que Steam reinicie...", now - last_launch));
                } else {
                    log("CEF debugging NO detectado, lanzando cef.bat...");
                    launch_cef_bat();
                }
            }

            // 2. Chequear/actualizar/lanzar backend.exe
            check_and_update_backend();
        });
    }
    1
}

// ── Version DLL exports ───────────────────────────────────────────────────────

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