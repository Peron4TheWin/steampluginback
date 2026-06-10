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
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut u8;
    fn TerminateProcess(handle: *mut u8, exit_code: u32) -> i32;
    fn CloseHandle(handle: *mut u8) -> i32;
}

#[link(name = "iphlpapi")]
extern "system" {
    fn GetExtendedTcpTable(
        table: *mut u8,
        size: *mut u32,
        order: i32,
        af: u32,
        table_class: u32,
        reserved: u32,
    ) -> u32;
}

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
fn log_path() -> String { format!("{}\\winhttp_dll.log", steam_dir()) }
fn backend_path() -> String { format!("{}\\{}", steam_dir(), BACKEND_EXE) }

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

// ── Winhttp proxy ─────────────────────────────────────────────────────────────

type Fn_WinHttpAddRequestHeaders        = unsafe extern "system" fn(*mut u8, *const u16, u32, u32) -> i32;
type Fn_WinHttpAddRequestHeadersEx      = unsafe extern "system" fn(*mut u8, u32, u64, u64, u32, *const u8, *const u8, *mut u32, *mut u32) -> u32;
type Fn_WinHttpCheckPlatform            = unsafe extern "system" fn() -> i32;
type Fn_WinHttpCloseHandle              = unsafe extern "system" fn(*mut u8) -> i32;
type Fn_WinHttpConnect                  = unsafe extern "system" fn(*mut u8, *const u16, u16, u32) -> *mut u8;
type Fn_WinHttpCrackUrl                 = unsafe extern "system" fn(*const u16, u32, u32, *mut u8) -> i32;
type Fn_WinHttpCreateUrl                = unsafe extern "system" fn(*mut u8, u32, *mut u16, *mut u32) -> i32;
type Fn_WinHttpDetectAutoProxyConfigUrl = unsafe extern "system" fn(u32, *mut *mut u16) -> i32;
type Fn_WinHttpGetDefaultProxyConfiguration = unsafe extern "system" fn(*mut u8) -> i32;
type Fn_WinHttpGetIEProxyConfigForCurrentUser = unsafe extern "system" fn(*mut u8) -> i32;
type Fn_WinHttpGetProxyForUrl           = unsafe extern "system" fn(*mut u8, *const u16, *mut u8, *mut u8) -> i32;
type Fn_WinHttpGetProxyForUrlEx         = unsafe extern "system" fn(*mut u8, *const u16, *mut u8, usize) -> u32;
type Fn_WinHttpGetProxyResult           = unsafe extern "system" fn(*mut u8, *mut u8) -> u32;
type Fn_WinHttpOpen                     = unsafe extern "system" fn(*const u16, u32, *const u16, *const u16, u32) -> *mut u8;
type Fn_WinHttpOpenRequest              = unsafe extern "system" fn(*mut u8, *const u16, *const u16, *const u16, *const u16, *const *const u16, u32) -> *mut u8;
type Fn_WinHttpQueryAuthSchemes         = unsafe extern "system" fn(*mut u8, *mut u32, *mut u32, *mut u32) -> i32;
type Fn_WinHttpQueryDataAvailable       = unsafe extern "system" fn(*mut u8, *mut u32) -> i32;
type Fn_WinHttpQueryHeaders             = unsafe extern "system" fn(*mut u8, u32, *const u16, *mut u8, *mut u32, *mut u32) -> i32;
type Fn_WinHttpQueryOption              = unsafe extern "system" fn(*mut u8, u32, *mut u8, *mut u32) -> i32;
type Fn_WinHttpReadData                 = unsafe extern "system" fn(*mut u8, *mut u8, u32, *mut u32) -> i32;
type Fn_WinHttpReceiveResponse          = unsafe extern "system" fn(*mut u8, *mut u8) -> i32;
type Fn_WinHttpResetAutoProxy           = unsafe extern "system" fn(*mut u8, u32) -> u32;
type Fn_WinHttpSendRequest              = unsafe extern "system" fn(*mut u8, *const u16, u32, *mut u8, u32, u32, usize) -> i32;
type Fn_WinHttpSetCredentials           = unsafe extern "system" fn(*mut u8, u32, u32, *const u16, *const u16, *mut u8) -> i32;
type Fn_WinHttpSetDefaultProxyConfiguration = unsafe extern "system" fn(*mut u8) -> i32;
type Fn_WinHttpSetOption                = unsafe extern "system" fn(*mut u8, u32, *mut u8, u32) -> i32;
type Fn_WinHttpSetStatusCallback        = unsafe extern "system" fn(*mut u8, *mut u8, u32, usize) -> *mut u8;
type Fn_WinHttpSetTimeouts              = unsafe extern "system" fn(*mut u8, i32, i32, i32, i32) -> i32;
type Fn_WinHttpTimeFromSystemTime       = unsafe extern "system" fn(*const u8, *mut u16) -> i32;
type Fn_WinHttpTimeToSystemTime         = unsafe extern "system" fn(*const u16, *mut u8) -> i32;
type Fn_WinHttpWriteData                = unsafe extern "system" fn(*mut u8, *const u8, u32, *mut u32) -> i32;
type Fn_WinHttpWebSocketClose           = unsafe extern "system" fn(*mut u8, u16, *const u8, u32) -> u32;
type Fn_WinHttpWebSocketCompleteUpgrade = unsafe extern "system" fn(*mut u8, usize) -> *mut u8;
type Fn_WinHttpWebSocketQueryCloseStatus = unsafe extern "system" fn(*mut u8, *mut u16, *mut u8, u32, *mut u32) -> u32;
type Fn_WinHttpWebSocketReceive         = unsafe extern "system" fn(*mut u8, *mut u8, u32, *mut u32, *mut u32) -> u32;
type Fn_WinHttpWebSocketSend            = unsafe extern "system" fn(*mut u8, u32, *mut u8, u32) -> u32;
type Fn_WinHttpWebSocketShutdown        = unsafe extern "system" fn(*mut u8, u16, *mut u8, u32) -> u32;
type Fn_SvchostPushServiceGlobals       = unsafe extern "system" fn(*mut u8) -> u32;

struct WinhttpFns {
    AddRequestHeaders:        Fn_WinHttpAddRequestHeaders,
    AddRequestHeadersEx:      Fn_WinHttpAddRequestHeadersEx,
    CheckPlatform:            Fn_WinHttpCheckPlatform,
    CloseHandle:              Fn_WinHttpCloseHandle,
    Connect:                  Fn_WinHttpConnect,
    CrackUrl:                 Fn_WinHttpCrackUrl,
    CreateUrl:                Fn_WinHttpCreateUrl,
    DetectAutoProxyConfigUrl: Fn_WinHttpDetectAutoProxyConfigUrl,
    GetDefaultProxyConfiguration: Fn_WinHttpGetDefaultProxyConfiguration,
    GetIEProxyConfigForCurrentUser: Fn_WinHttpGetIEProxyConfigForCurrentUser,
    GetProxyForUrl:           Fn_WinHttpGetProxyForUrl,
    GetProxyForUrlEx:         Fn_WinHttpGetProxyForUrlEx,
    GetProxyResult:           Fn_WinHttpGetProxyResult,
    Open:                     Fn_WinHttpOpen,
    OpenRequest:              Fn_WinHttpOpenRequest,
    QueryAuthSchemes:         Fn_WinHttpQueryAuthSchemes,
    QueryDataAvailable:       Fn_WinHttpQueryDataAvailable,
    QueryHeaders:             Fn_WinHttpQueryHeaders,
    QueryOption:              Fn_WinHttpQueryOption,
    ReadData:                 Fn_WinHttpReadData,
    ReceiveResponse:          Fn_WinHttpReceiveResponse,
    ResetAutoProxy:           Fn_WinHttpResetAutoProxy,
    SendRequest:              Fn_WinHttpSendRequest,
    SetCredentials:           Fn_WinHttpSetCredentials,
    SetDefaultProxyConfiguration: Fn_WinHttpSetDefaultProxyConfiguration,
    SetOption:                Fn_WinHttpSetOption,
    SetStatusCallback:        Fn_WinHttpSetStatusCallback,
    SetTimeouts:              Fn_WinHttpSetTimeouts,
    TimeFromSystemTime:       Fn_WinHttpTimeFromSystemTime,
    TimeToSystemTime:         Fn_WinHttpTimeToSystemTime,
    WriteData:                Fn_WinHttpWriteData,
    WebSocketClose:           Fn_WinHttpWebSocketClose,
    WebSocketCompleteUpgrade: Fn_WinHttpWebSocketCompleteUpgrade,
    WebSocketQueryCloseStatus: Fn_WinHttpWebSocketQueryCloseStatus,
    WebSocketReceive:         Fn_WinHttpWebSocketReceive,
    WebSocketSend:            Fn_WinHttpWebSocketSend,
    WebSocketShutdown:        Fn_WinHttpWebSocketShutdown,
    SvchostPushServiceGlobals: Fn_SvchostPushServiceGlobals,
}

unsafe impl Send for WinhttpFns {}
unsafe impl Sync for WinhttpFns {}

static REAL: OnceLock<WinhttpFns> = OnceLock::new();

extern "system" {
    fn LoadLibraryA(name: *const i8) -> *mut u8;
    fn GetProcAddress(module: *mut u8, name: *const i8) -> *mut u8;
}

fn load_real_winhttp() {
    unsafe {
        let path = format!("{}\\winhttp.dll\0", SYSTEM32_DIR.get().unwrap());
        let lib = LoadLibraryA(path.as_ptr() as _);
        if lib.is_null() { log("ERROR: no se pudo cargar winhttp.dll real"); return; }
        macro_rules! gfn {
            ($name:ident) => {{
                let ptr = GetProcAddress(lib, concat!(stringify!($name), "\0").as_ptr() as _);
                std::mem::transmute(ptr)
            }};
        }
        REAL.set(WinhttpFns {
            AddRequestHeaders:        gfn!(WinHttpAddRequestHeaders),
            AddRequestHeadersEx:      gfn!(WinHttpAddRequestHeadersEx),
            CheckPlatform:            gfn!(WinHttpCheckPlatform),
            CloseHandle:              gfn!(WinHttpCloseHandle),
            Connect:                  gfn!(WinHttpConnect),
            CrackUrl:                 gfn!(WinHttpCrackUrl),
            CreateUrl:                gfn!(WinHttpCreateUrl),
            DetectAutoProxyConfigUrl: gfn!(WinHttpDetectAutoProxyConfigUrl),
            GetDefaultProxyConfiguration: gfn!(WinHttpGetDefaultProxyConfiguration),
            GetIEProxyConfigForCurrentUser: gfn!(WinHttpGetIEProxyConfigForCurrentUser),
            GetProxyForUrl:           gfn!(WinHttpGetProxyForUrl),
            GetProxyForUrlEx:         gfn!(WinHttpGetProxyForUrlEx),
            GetProxyResult:           gfn!(WinHttpGetProxyResult),
            Open:                     gfn!(WinHttpOpen),
            OpenRequest:              gfn!(WinHttpOpenRequest),
            QueryAuthSchemes:         gfn!(WinHttpQueryAuthSchemes),
            QueryDataAvailable:       gfn!(WinHttpQueryDataAvailable),
            QueryHeaders:             gfn!(WinHttpQueryHeaders),
            QueryOption:              gfn!(WinHttpQueryOption),
            ReadData:                 gfn!(WinHttpReadData),
            ReceiveResponse:          gfn!(WinHttpReceiveResponse),
            ResetAutoProxy:           gfn!(WinHttpResetAutoProxy),
            SendRequest:              gfn!(WinHttpSendRequest),
            SetCredentials:           gfn!(WinHttpSetCredentials),
            SetDefaultProxyConfiguration: gfn!(WinHttpSetDefaultProxyConfiguration),
            SetOption:                gfn!(WinHttpSetOption),
            SetStatusCallback:        gfn!(WinHttpSetStatusCallback),
            SetTimeouts:              gfn!(WinHttpSetTimeouts),
            TimeFromSystemTime:       gfn!(WinHttpTimeFromSystemTime),
            TimeToSystemTime:         gfn!(WinHttpTimeToSystemTime),
            WriteData:                gfn!(WinHttpWriteData),
            WebSocketClose:           gfn!(WinHttpWebSocketClose),
            WebSocketCompleteUpgrade: gfn!(WinHttpWebSocketCompleteUpgrade),
            WebSocketQueryCloseStatus: gfn!(WinHttpWebSocketQueryCloseStatus),
            WebSocketReceive:         gfn!(WinHttpWebSocketReceive),
            WebSocketSend:            gfn!(WinHttpWebSocketSend),
            WebSocketShutdown:        gfn!(WinHttpWebSocketShutdown),
            SvchostPushServiceGlobals: gfn!(SvchostPushServiceGlobals),
        }).ok();
        log("winhttp.dll real cargado OK");
    }
}

// ── SHA256 ────────────────────────────────────────────────────────────────────

fn sha256_file(path: &str) -> Option<String> {
use std::io::BufReader;
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
Some(format!("{:x}", hasher.finalize()))
}

// ── GitHub release check + download ──────────────────────────────────────────

fn extract_str_after<'a>(json: &'a str, key: &str) -> Option<&'a str> {
let needle = format!("\"{}\":", key);
let start = json.find(&needle)? + needle.len();
let rest = json[start..].trim_start();
if !rest.starts_with('"') { return None; }
let inner = &rest[1..];
Some(&inner[..inner.find('"')?])
}

fn find_exe_asset(json: &str) -> Option<(String, String)> {
let mut search = json;
while let Some(pos) = search.find("\"name\":") {
let rest = &search[pos..];
if let Some(name) = extract_str_after(rest, "name") {
if name.ends_with(".exe") {
let url = extract_str_after(rest, "browser_download_url").unwrap_or("").to_string();
let sha = extract_str_after(rest, "digest")
.and_then(|d| d.strip_prefix("sha256:"))
.unwrap_or("").to_lowercase();
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
        .set("User-Agent", "winhttp-dll/1.0")
        .set("Accept", "application/vnd.github+json")
        .call()
    {
        Ok(resp) => { let mut s = String::new(); resp.into_reader().read_to_string(&mut s).unwrap_or(0); s }
        Err(e) => {
    log(&format!("ERROR consultando GitHub API: {}", e));
    kill_port_3000();
    launch_backend();
    return;
    }
    };

    let (exe_url, expected_sha) = match find_exe_asset(&release_json) {
        Some(v) => v,
        None => {
        log("ERROR: no se encontro asset .exe en el latest release");
        kill_port_3000();
        launch_backend();
        return;
        }
    };
    log(&format!("Asset: {} | SHA256 esperado: {}", exe_url, expected_sha));

    if expected_sha.is_empty() {
        log("WARN: el asset no tiene campo digest, no se puede verificar SHA256");
        kill_port_3000();
        launch_backend();
        return;
    }

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
        log("SHA256 no coincide, matando proceso y descargando nueva version...");
        kill_port_3000();
        std::thread::sleep(std::time::Duration::from_millis(1500));

        if Path::new(&local_path).exists() {
            if let Err(e) = fs::remove_file(&local_path) {
                log(&format!("ERROR borrando backend.exe viejo: {}", e));
                return;
            }
            log("backend.exe viejo borrado");
        }

        match ureq::get(&exe_url).set("User-Agent", "winhttp-dll/1.0").call() {
            Ok(resp) => {
            let mut buf = Vec::new();
            resp.into_reader().read_to_end(&mut buf).ok();
            match fs::write(&local_path, &buf) {
            Ok(_) => log(&format!("backend.exe descargado ({} bytes)", buf.len())),
            Err(e) => { log(&format!("ERROR escribiendo backend.exe: {}", e)); return; }
            }
            let new_sha = sha256_file(&local_path).unwrap_or_default();
            if new_sha == expected_sha {
            log("SHA256 verificado OK post-descarga");
            } else {
            log(&format!("WARN: SHA256 post-descarga no coincide! ({})", new_sha));
            }
            }
            Err(e) => { log(&format!("ERROR descargando backend.exe: {}", e)); return; }
        }

        launch_backend();
        return;
    }

    kill_port_3000();
    launch_backend();
}

fn kill_port_3000() {
    const TCP_TABLE_OWNER_PID_ALL: u32 = 5;
    const AF_INET: u32 = 2;
    const PROCESS_TERMINATE: u32 = 0x0001;
    unsafe {
        let mut size: u32 = 0;
        GetExtendedTcpTable(std::ptr::null_mut(), &mut size, 0, AF_INET, TCP_TABLE_OWNER_PID_ALL, 0);
        if size == 0 { return; }
        let mut buf = vec![0u8; size as usize];
        if GetExtendedTcpTable(buf.as_mut_ptr(), &mut size, 0, AF_INET, TCP_TABLE_OWNER_PID_ALL, 0) != 0 { return; }
        let num_entries = u32::from_le_bytes(buf[0..4].try_into().unwrap()) as usize;
        const ROW_SIZE: usize = 24;
        for i in 0..num_entries {
            let offset = 4 + i * ROW_SIZE;
            if offset + ROW_SIZE > buf.len() { break; }
            let raw_port = u32::from_le_bytes(buf[offset+8..offset+12].try_into().unwrap());
            let port = ((raw_port & 0xFF) << 8) | ((raw_port >> 8) & 0xFF);
            if port != 3000 { continue; }
            let pid = u32::from_le_bytes(buf[offset+20..offset+24].try_into().unwrap());
            log(&format!("Puerto 3000 en uso por PID {}, matando...", pid));
            let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if !handle.is_null() {
                TerminateProcess(handle, 1);
                CloseHandle(handle);
                log(&format!("PID {} terminado", pid));
            } else {
                log(&format!("ERROR: no se pudo abrir PID {}", pid));
            }
        }
    }
}

fn launch_backend() {
    let path = backend_path();
    if !Path::new(&path).exists() { log("backend.exe no existe, no se puede lanzar"); return; }
    fn to_wide(s: &str) -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() }
    let mut si = vec![0u8; 104];
    si[0] = 104;
    let mut pi = vec![0u8; 24];
    let app = to_wide(&path);
    let mut cmd = to_wide(&format!("\"{}\"", path));
    let dir = to_wide(steam_dir());
    unsafe {
        let result = CreateProcessW(
            app.as_ptr(), cmd.as_mut_ptr(),
            std::ptr::null_mut(), std::ptr::null_mut(),
            0, DETACHED_PROCESS | CREATE_NO_WINDOW,
            std::ptr::null_mut(), dir.as_ptr(),
            si.as_mut_ptr(), pi.as_mut_ptr(),
        );
        if result != 0 { log("backend.exe lanzado en background OK"); }
        else { log("ERROR: CreateProcessW fallo al lanzar backend.exe"); }
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
            load_real_winhttp();
            check_and_update_backend();
        });
    }
    1
}

// ── Winhttp exports ───────────────────────────────────────────────────────────

#[no_mangle] pub unsafe extern "system" fn WinHttpAddRequestHeaders(a: *mut u8, b: *const u16, c: u32, d: u32) -> i32 { (REAL.get().unwrap().AddRequestHeaders)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn WinHttpAddRequestHeadersEx(a: *mut u8, b: u32, c: u64, d: u64, e: u32, f: *const u8, g: *const u8, h: *mut u32, i: *mut u32) -> u32 { (REAL.get().unwrap().AddRequestHeadersEx)(a, b, c, d, e, f, g, h, i) }
#[no_mangle] pub unsafe extern "system" fn WinHttpCheckPlatform() -> i32 { (REAL.get().unwrap().CheckPlatform)() }
#[no_mangle] pub unsafe extern "system" fn WinHttpCloseHandle(a: *mut u8) -> i32 { (REAL.get().unwrap().CloseHandle)(a) }
#[no_mangle] pub unsafe extern "system" fn WinHttpConnect(a: *mut u8, b: *const u16, c: u16, d: u32) -> *mut u8 { (REAL.get().unwrap().Connect)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn WinHttpCrackUrl(a: *const u16, b: u32, c: u32, d: *mut u8) -> i32 { (REAL.get().unwrap().CrackUrl)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn WinHttpCreateUrl(a: *mut u8, b: u32, c: *mut u16, d: *mut u32) -> i32 { (REAL.get().unwrap().CreateUrl)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn WinHttpDetectAutoProxyConfigUrl(a: u32, b: *mut *mut u16) -> i32 { (REAL.get().unwrap().DetectAutoProxyConfigUrl)(a, b) }
#[no_mangle] pub unsafe extern "system" fn WinHttpGetDefaultProxyConfiguration(a: *mut u8) -> i32 { (REAL.get().unwrap().GetDefaultProxyConfiguration)(a) }
#[no_mangle] pub unsafe extern "system" fn WinHttpGetIEProxyConfigForCurrentUser(a: *mut u8) -> i32 { (REAL.get().unwrap().GetIEProxyConfigForCurrentUser)(a) }
#[no_mangle] pub unsafe extern "system" fn WinHttpGetProxyForUrl(a: *mut u8, b: *const u16, c: *mut u8, d: *mut u8) -> i32 { (REAL.get().unwrap().GetProxyForUrl)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn WinHttpGetProxyForUrlEx(a: *mut u8, b: *const u16, c: *mut u8, d: usize) -> u32 { (REAL.get().unwrap().GetProxyForUrlEx)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn WinHttpGetProxyResult(a: *mut u8, b: *mut u8) -> u32 { (REAL.get().unwrap().GetProxyResult)(a, b) }
#[no_mangle] pub unsafe extern "system" fn WinHttpOpen(a: *const u16, b: u32, c: *const u16, d: *const u16, e: u32) -> *mut u8 { (REAL.get().unwrap().Open)(a, b, c, d, e) }
#[no_mangle] pub unsafe extern "system" fn WinHttpOpenRequest(a: *mut u8, b: *const u16, c: *const u16, d: *const u16, e: *const u16, f: *const *const u16, g: u32) -> *mut u8 { (REAL.get().unwrap().OpenRequest)(a, b, c, d, e, f, g) }
#[no_mangle] pub unsafe extern "system" fn WinHttpQueryAuthSchemes(a: *mut u8, b: *mut u32, c: *mut u32, d: *mut u32) -> i32 { (REAL.get().unwrap().QueryAuthSchemes)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn WinHttpQueryDataAvailable(a: *mut u8, b: *mut u32) -> i32 { (REAL.get().unwrap().QueryDataAvailable)(a, b) }
#[no_mangle] pub unsafe extern "system" fn WinHttpQueryHeaders(a: *mut u8, b: u32, c: *const u16, d: *mut u8, e: *mut u32, f: *mut u32) -> i32 { (REAL.get().unwrap().QueryHeaders)(a, b, c, d, e, f) }
#[no_mangle] pub unsafe extern "system" fn WinHttpQueryOption(a: *mut u8, b: u32, c: *mut u8, d: *mut u32) -> i32 { (REAL.get().unwrap().QueryOption)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn WinHttpReadData(a: *mut u8, b: *mut u8, c: u32, d: *mut u32) -> i32 { (REAL.get().unwrap().ReadData)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn WinHttpReceiveResponse(a: *mut u8, b: *mut u8) -> i32 { (REAL.get().unwrap().ReceiveResponse)(a, b) }
#[no_mangle] pub unsafe extern "system" fn WinHttpResetAutoProxy(a: *mut u8, b: u32) -> u32 { (REAL.get().unwrap().ResetAutoProxy)(a, b) }
#[no_mangle] pub unsafe extern "system" fn WinHttpSendRequest(a: *mut u8, b: *const u16, c: u32, d: *mut u8, e: u32, f: u32, g: usize) -> i32 { (REAL.get().unwrap().SendRequest)(a, b, c, d, e, f, g) }
#[no_mangle] pub unsafe extern "system" fn WinHttpSetCredentials(a: *mut u8, b: u32, c: u32, d: *const u16, e: *const u16, f: *mut u8) -> i32 { (REAL.get().unwrap().SetCredentials)(a, b, c, d, e, f) }
#[no_mangle] pub unsafe extern "system" fn WinHttpSetDefaultProxyConfiguration(a: *mut u8) -> i32 { (REAL.get().unwrap().SetDefaultProxyConfiguration)(a) }
#[no_mangle] pub unsafe extern "system" fn WinHttpSetOption(a: *mut u8, b: u32, c: *mut u8, d: u32) -> i32 { (REAL.get().unwrap().SetOption)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn WinHttpSetStatusCallback(a: *mut u8, b: *mut u8, c: u32, d: usize) -> *mut u8 { (REAL.get().unwrap().SetStatusCallback)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn WinHttpSetTimeouts(a: *mut u8, b: i32, c: i32, d: i32, e: i32) -> i32 { (REAL.get().unwrap().SetTimeouts)(a, b, c, d, e) }
#[no_mangle] pub unsafe extern "system" fn WinHttpTimeFromSystemTime(a: *const u8, b: *mut u16) -> i32 { (REAL.get().unwrap().TimeFromSystemTime)(a, b) }
#[no_mangle] pub unsafe extern "system" fn WinHttpTimeToSystemTime(a: *const u16, b: *mut u8) -> i32 { (REAL.get().unwrap().TimeToSystemTime)(a, b) }
#[no_mangle] pub unsafe extern "system" fn WinHttpWriteData(a: *mut u8, b: *const u8, c: u32, d: *mut u32) -> i32 { (REAL.get().unwrap().WriteData)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn WinHttpWebSocketClose(a: *mut u8, b: u16, c: *const u8, d: u32) -> u32 { (REAL.get().unwrap().WebSocketClose)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn WinHttpWebSocketCompleteUpgrade(a: *mut u8, b: usize) -> *mut u8 { (REAL.get().unwrap().WebSocketCompleteUpgrade)(a, b) }
#[no_mangle] pub unsafe extern "system" fn WinHttpWebSocketQueryCloseStatus(a: *mut u8, b: *mut u16, c: *mut u8, d: u32, e: *mut u32) -> u32 { (REAL.get().unwrap().WebSocketQueryCloseStatus)(a, b, c, d, e) }
#[no_mangle] pub unsafe extern "system" fn WinHttpWebSocketReceive(a: *mut u8, b: *mut u8, c: u32, d: *mut u32, e: *mut u32) -> u32 { (REAL.get().unwrap().WebSocketReceive)(a, b, c, d, e) }
#[no_mangle] pub unsafe extern "system" fn WinHttpWebSocketSend(a: *mut u8, b: u32, c: *mut u8, d: u32) -> u32 { (REAL.get().unwrap().WebSocketSend)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn WinHttpWebSocketShutdown(a: *mut u8, b: u16, c: *mut u8, d: u32) -> u32 { (REAL.get().unwrap().WebSocketShutdown)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn SvchostPushServiceGlobals(a: *mut u8) -> u32 { (REAL.get().unwrap().SvchostPushServiceGlobals)(a) }
