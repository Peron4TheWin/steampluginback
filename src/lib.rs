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
    fn GetModuleFileNameA(hmodule: *mut u8, filename: *mut i8, size: u32) -> u32;
    fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
    fn LoadLibraryW(name: *const u16) -> *mut u8;
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
fn log_path() -> String { format!("{}\\wsock32_dll.log", steam_dir()) }
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

// ── Wsock32 proxy type aliases ────────────────────────────────────────────────

type Fn_accept                     = unsafe extern "system" fn(usize, *mut u8, *mut i32) -> usize;
type Fn_bind                       = unsafe extern "system" fn(usize, *const u8, i32) -> i32;
type Fn_closesocket                = unsafe extern "system" fn(usize) -> i32;
type Fn_connect                    = unsafe extern "system" fn(usize, *const u8, i32) -> i32;
type Fn_getpeername                = unsafe extern "system" fn(usize, *mut u8, *mut i32) -> i32;
type Fn_getsockname                = unsafe extern "system" fn(usize, *mut u8, *mut i32) -> i32;
type Fn_getsockopt                 = unsafe extern "system" fn(usize, i32, i32, *mut u8, *mut i32) -> i32;
type Fn_htonl                      = unsafe extern "system" fn(u32) -> u32;
type Fn_htons                      = unsafe extern "system" fn(u16) -> u16;
type Fn_inet_addr                  = unsafe extern "system" fn(*const i8) -> u32;
type Fn_inet_ntoa                  = unsafe extern "system" fn(u32) -> *mut i8;
type Fn_ioctlsocket                = unsafe extern "system" fn(usize, i32, *mut u32) -> i32;
type Fn_listen                     = unsafe extern "system" fn(usize, i32) -> i32;
type Fn_ntohl                      = unsafe extern "system" fn(u32) -> u32;
type Fn_ntohs                      = unsafe extern "system" fn(u16) -> u16;
type Fn_recv                       = unsafe extern "system" fn(usize, *mut u8, i32, i32) -> i32;
type Fn_recvfrom                   = unsafe extern "system" fn(usize, *mut u8, i32, i32, *mut u8, *mut i32) -> i32;
type Fn_select                     = unsafe extern "system" fn(i32, *mut u8, *mut u8, *mut u8, *const u8) -> i32;
type Fn_send                       = unsafe extern "system" fn(usize, *const u8, i32, i32) -> i32;
type Fn_sendto                     = unsafe extern "system" fn(usize, *const u8, i32, i32, *const u8, i32) -> i32;
type Fn_setsockopt                 = unsafe extern "system" fn(usize, i32, i32, *const u8, i32) -> i32;
type Fn_shutdown                   = unsafe extern "system" fn(usize, i32) -> i32;
type Fn_socket                     = unsafe extern "system" fn(i32, i32, i32) -> usize;
type Fn_MigrateWinsockConfiguration = unsafe extern "system" fn() -> ();
type Fn_gethostbyaddr              = unsafe extern "system" fn(*const u8, i32, i32) -> *mut u8;
type Fn_gethostbyname              = unsafe extern "system" fn(*const i8) -> *mut u8;
type Fn_getprotobyname             = unsafe extern "system" fn(*const i8) -> *mut u8;
type Fn_getprotobynumber           = unsafe extern "system" fn(i32) -> *mut u8;
type Fn_getservbyname              = unsafe extern "system" fn(*const i8, *const i8) -> *mut u8;
type Fn_getservbyport              = unsafe extern "system" fn(i32, *const i8) -> *mut u8;
type Fn_gethostname                = unsafe extern "system" fn(*mut i8, i32) -> i32;
type Fn_WSAAsyncSelect             = unsafe extern "system" fn(usize, *mut u8, u32, i32) -> i32;
type Fn_WSAAsyncGetHostByAddr      = unsafe extern "system" fn(*mut u8, u32, *const u8, i32, i32, *mut i8, i32) -> *mut u8;
type Fn_WSAAsyncGetHostByName      = unsafe extern "system" fn(*mut u8, u32, *const i8, *mut i8, i32) -> *mut u8;
type Fn_WSAAsyncGetProtoByNumber   = unsafe extern "system" fn(*mut u8, u32, i32, *mut i8, i32) -> *mut u8;
type Fn_WSAAsyncGetProtoByName     = unsafe extern "system" fn(*mut u8, u32, *const i8, *mut i8, i32) -> *mut u8;
type Fn_WSAAsyncGetServByPort      = unsafe extern "system" fn(*mut u8, u32, i32, *const i8, *mut i8, i32) -> *mut u8;
type Fn_WSAAsyncGetServByName      = unsafe extern "system" fn(*mut u8, u32, *const i8, *const i8, *mut i8, i32) -> *mut u8;
type Fn_WSACancelAsyncRequest      = unsafe extern "system" fn(*mut u8) -> i32;
type Fn_WSASetBlockingHook         = unsafe extern "system" fn(*mut u8) -> *mut u8;
type Fn_WSAUnhookBlockingHook      = unsafe extern "system" fn() -> i32;
type Fn_WSAGetLastError            = unsafe extern "system" fn() -> i32;
type Fn_WSASetLastError            = unsafe extern "system" fn(i32) -> ();
type Fn_WSACancelBlockingCall      = unsafe extern "system" fn() -> i32;
type Fn_WSAIsBlocking              = unsafe extern "system" fn() -> i32;
type Fn_WSAStartup                 = unsafe extern "system" fn(u16, *mut u8) -> i32;
type Fn_WSACleanup                 = unsafe extern "system" fn() -> i32;
type Fn___WSAFDIsSet               = unsafe extern "system" fn(usize, *mut u8) -> i32;
type Fn_WEP                        = unsafe extern "system" fn() -> ();
type Fn_WSApSetPostRoutine         = unsafe extern "system" fn(*mut u8) -> i32;
type Fn_inet_network               = unsafe extern "system" fn(*const i8) -> u32;
type Fn_getnetbyname               = unsafe extern "system" fn(*const i8) -> *mut u8;
type Fn_rcmd                       = unsafe extern "system" fn(*mut *mut u8, u16, *const i8, *const i8, *const i8, *mut i32) -> i32;
type Fn_rexec                      = unsafe extern "system" fn(*mut *mut u8, u16, *const i8, *const i8, *const i8, *mut i32) -> i32;
type Fn_rresvport                  = unsafe extern "system" fn(*mut i32) -> i32;
type Fn_sethostname                = unsafe extern "system" fn(*mut i8, i32) -> i32;
type Fn_dn_expand                  = unsafe extern "system" fn(*const u8, *const u8, *const u8, *mut i8, i32) -> i32;
type Fn_WSARecvEx                  = unsafe extern "system" fn(usize, *mut u8, i32, *mut i32) -> i32;
type Fn_s_perror                   = unsafe extern "system" fn(*const i8) -> ();
type Fn_GetAddressByNameA          = unsafe extern "system" fn(*const u8, *const i8, *const i8, *const i8, *const i8, i32, i32, *const u8, i32, *mut u8, *mut i32) -> i32;
type Fn_GetAddressByNameW          = unsafe extern "system" fn(*const u8, *const u16, *const u16, *const u16, *const u16, i32, i32, *const u8, i32, *mut u8, *mut i32) -> i32;
type Fn_EnumProtocolsA             = unsafe extern "system" fn(*mut i32, *mut u8, *mut u32) -> i32;
type Fn_EnumProtocolsW             = unsafe extern "system" fn(*mut i32, *mut u8, *mut u32) -> i32;
type Fn_GetTypeByNameA             = unsafe extern "system" fn(*const i8, *mut u8) -> i32;
type Fn_GetTypeByNameW             = unsafe extern "system" fn(*const u16, *mut u8) -> i32;
type Fn_GetNameByTypeA             = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> i32;
type Fn_GetNameByTypeW             = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> i32;
type Fn_SetServiceA                = unsafe extern "system" fn(*mut u8, u32, u32, u32) -> i32;
type Fn_SetServiceW                = unsafe extern "system" fn(*mut u8, u32, u32, u32) -> i32;
type Fn_GetServiceA                = unsafe extern "system" fn(*mut u8, *mut u8, *mut u8, *mut u32) -> i32;
type Fn_GetServiceW                = unsafe extern "system" fn(*mut u8, *mut u8, *mut u8, *mut u32) -> i32;
type Fn_NPLoadNameSpaces           = unsafe extern "system" fn(*mut u8, *mut u8, *mut u8) -> u32;
type Fn_TransmitFile               = unsafe extern "system" fn(usize, *mut u8, u32, u32, *mut u8, *mut u8, u32) -> i32;
type Fn_AcceptEx                   = unsafe extern "system" fn(usize, usize, *mut u8, u32, u32, u32, *mut u32, *mut u8) -> i32;
type Fn_GetAcceptExSockaddrs       = unsafe extern "system" fn(*mut u8, u32, u32, u32, *mut *mut u8, *mut i32, *mut *mut u8, *mut i32) -> ();

struct Wsock32Fns {
    accept:                     Fn_accept,
    bind:                       Fn_bind,
    closesocket:                Fn_closesocket,
    connect:                    Fn_connect,
    getpeername:                Fn_getpeername,
    getsockname:                Fn_getsockname,
    getsockopt:                 Fn_getsockopt,
    htonl:                      Fn_htonl,
    htons:                      Fn_htons,
    inet_addr:                  Fn_inet_addr,
    inet_ntoa:                  Fn_inet_ntoa,
    ioctlsocket:                Fn_ioctlsocket,
    listen:                     Fn_listen,
    ntohl:                      Fn_ntohl,
    ntohs:                      Fn_ntohs,
    recv:                       Fn_recv,
    recvfrom:                   Fn_recvfrom,
    select:                     Fn_select,
    send:                       Fn_send,
    sendto:                     Fn_sendto,
    setsockopt:                 Fn_setsockopt,
    shutdown:                   Fn_shutdown,
    socket:                     Fn_socket,
    MigrateWinsockConfiguration: Fn_MigrateWinsockConfiguration,
    gethostbyaddr:              Fn_gethostbyaddr,
    gethostbyname:              Fn_gethostbyname,
    getprotobyname:             Fn_getprotobyname,
    getprotobynumber:           Fn_getprotobynumber,
    getservbyname:              Fn_getservbyname,
    getservbyport:              Fn_getservbyport,
    gethostname:                Fn_gethostname,
    WSAAsyncSelect:             Fn_WSAAsyncSelect,
    WSAAsyncGetHostByAddr:      Fn_WSAAsyncGetHostByAddr,
    WSAAsyncGetHostByName:      Fn_WSAAsyncGetHostByName,
    WSAAsyncGetProtoByNumber:   Fn_WSAAsyncGetProtoByNumber,
    WSAAsyncGetProtoByName:     Fn_WSAAsyncGetProtoByName,
    WSAAsyncGetServByPort:      Fn_WSAAsyncGetServByPort,
    WSAAsyncGetServByName:      Fn_WSAAsyncGetServByName,
    WSACancelAsyncRequest:      Fn_WSACancelAsyncRequest,
    WSASetBlockingHook:         Fn_WSASetBlockingHook,
    WSAUnhookBlockingHook:      Fn_WSAUnhookBlockingHook,
    WSAGetLastError:            Fn_WSAGetLastError,
    WSASetLastError:            Fn_WSASetLastError,
    WSACancelBlockingCall:      Fn_WSACancelBlockingCall,
    WSAIsBlocking:              Fn_WSAIsBlocking,
    WSAStartup:                 Fn_WSAStartup,
    WSACleanup:                 Fn_WSACleanup,
    __WSAFDIsSet:               Fn___WSAFDIsSet,
    WEP:                        Fn_WEP,
    WSApSetPostRoutine:         Fn_WSApSetPostRoutine,
    inet_network:               Fn_inet_network,
    getnetbyname:               Fn_getnetbyname,
    rcmd:                       Fn_rcmd,
    rexec:                      Fn_rexec,
    rresvport:                  Fn_rresvport,
    sethostname:                Fn_sethostname,
    dn_expand:                  Fn_dn_expand,
    WSARecvEx:                  Fn_WSARecvEx,
    s_perror:                   Fn_s_perror,
    GetAddressByNameA:          Fn_GetAddressByNameA,
    GetAddressByNameW:          Fn_GetAddressByNameW,
    EnumProtocolsA:             Fn_EnumProtocolsA,
    EnumProtocolsW:             Fn_EnumProtocolsW,
    GetTypeByNameA:             Fn_GetTypeByNameA,
    GetTypeByNameW:             Fn_GetTypeByNameW,
    GetNameByTypeA:             Fn_GetNameByTypeA,
    GetNameByTypeW:             Fn_GetNameByTypeW,
    SetServiceA:                Fn_SetServiceA,
    SetServiceW:                Fn_SetServiceW,
    GetServiceA:                Fn_GetServiceA,
    GetServiceW:                Fn_GetServiceW,
    NPLoadNameSpaces:           Fn_NPLoadNameSpaces,
    TransmitFile:               Fn_TransmitFile,
    AcceptEx:                   Fn_AcceptEx,
    GetAcceptExSockaddrs:       Fn_GetAcceptExSockaddrs,
}

unsafe impl Send for Wsock32Fns {}
unsafe impl Sync for Wsock32Fns {}

static REAL: OnceLock<Wsock32Fns> = OnceLock::new();

extern "system" {
    fn LoadLibraryA(name: *const i8) -> *mut u8;
    fn GetProcAddress(module: *mut u8, name: *const i8) -> *mut u8;
}

fn load_real_wsock32() {
    unsafe {
        let path = format!("{}\\wsock32.dll\0", SYSTEM32_DIR.get().unwrap());
        let lib = LoadLibraryA(path.as_ptr() as _);
        if lib.is_null() { log("ERROR: no se pudo cargar wsock32.dll real"); return; }
        macro_rules! gfn {
            ($name:expr) => {{
                let ptr = GetProcAddress(lib, concat!($name, "\0").as_ptr() as _);
                std::mem::transmute(ptr)
            }};
        }
        REAL.set(Wsock32Fns {
            accept:                     gfn!("accept"),
            bind:                       gfn!("bind"),
            closesocket:                gfn!("closesocket"),
            connect:                    gfn!("connect"),
            getpeername:                gfn!("getpeername"),
            getsockname:                gfn!("getsockname"),
            getsockopt:                 gfn!("getsockopt"),
            htonl:                      gfn!("htonl"),
            htons:                      gfn!("htons"),
            inet_addr:                  gfn!("inet_addr"),
            inet_ntoa:                  gfn!("inet_ntoa"),
            ioctlsocket:                gfn!("ioctlsocket"),
            listen:                     gfn!("listen"),
            ntohl:                      gfn!("ntohl"),
            ntohs:                      gfn!("ntohs"),
            recv:                       gfn!("recv"),
            recvfrom:                   gfn!("recvfrom"),
            select:                     gfn!("select"),
            send:                       gfn!("send"),
            sendto:                     gfn!("sendto"),
            setsockopt:                 gfn!("setsockopt"),
            shutdown:                   gfn!("shutdown"),
            socket:                     gfn!("socket"),
            MigrateWinsockConfiguration: gfn!("MigrateWinsockConfiguration"),
            gethostbyaddr:              gfn!("gethostbyaddr"),
            gethostbyname:              gfn!("gethostbyname"),
            getprotobyname:             gfn!("getprotobyname"),
            getprotobynumber:           gfn!("getprotobynumber"),
            getservbyname:              gfn!("getservbyname"),
            getservbyport:              gfn!("getservbyport"),
            gethostname:                gfn!("gethostname"),
            WSAAsyncSelect:             gfn!("WSAAsyncSelect"),
            WSAAsyncGetHostByAddr:      gfn!("WSAAsyncGetHostByAddr"),
            WSAAsyncGetHostByName:      gfn!("WSAAsyncGetHostByName"),
            WSAAsyncGetProtoByNumber:   gfn!("WSAAsyncGetProtoByNumber"),
            WSAAsyncGetProtoByName:     gfn!("WSAAsyncGetProtoByName"),
            WSAAsyncGetServByPort:      gfn!("WSAAsyncGetServByPort"),
            WSAAsyncGetServByName:      gfn!("WSAAsyncGetServByName"),
            WSACancelAsyncRequest:      gfn!("WSACancelAsyncRequest"),
            WSASetBlockingHook:         gfn!("WSASetBlockingHook"),
            WSAUnhookBlockingHook:      gfn!("WSAUnhookBlockingHook"),
            WSAGetLastError:            gfn!("WSAGetLastError"),
            WSASetLastError:            gfn!("WSASetLastError"),
            WSACancelBlockingCall:      gfn!("WSACancelBlockingCall"),
            WSAIsBlocking:              gfn!("WSAIsBlocking"),
            WSAStartup:                 gfn!("WSAStartup"),
            WSACleanup:                 gfn!("WSACleanup"),
            __WSAFDIsSet:               gfn!("__WSAFDIsSet"),
            WEP:                        gfn!("WEP"),
            WSApSetPostRoutine:         gfn!("WSApSetPostRoutine"),
            inet_network:               gfn!("inet_network"),
            getnetbyname:               gfn!("getnetbyname"),
            rcmd:                       gfn!("rcmd"),
            rexec:                      gfn!("rexec"),
            rresvport:                  gfn!("rresvport"),
            sethostname:                gfn!("sethostname"),
            dn_expand:                  gfn!("dn_expand"),
            WSARecvEx:                  gfn!("WSARecvEx"),
            s_perror:                   gfn!("s_perror"),
            GetAddressByNameA:          gfn!("GetAddressByNameA"),
            GetAddressByNameW:          gfn!("GetAddressByNameW"),
            EnumProtocolsA:             gfn!("EnumProtocolsA"),
            EnumProtocolsW:             gfn!("EnumProtocolsW"),
            GetTypeByNameA:             gfn!("GetTypeByNameA"),
            GetTypeByNameW:             gfn!("GetTypeByNameW"),
            GetNameByTypeA:             gfn!("GetNameByTypeA"),
            GetNameByTypeW:             gfn!("GetNameByTypeW"),
            SetServiceA:                gfn!("SetServiceA"),
            SetServiceW:                gfn!("SetServiceW"),
            GetServiceA:                gfn!("GetServiceA"),
            GetServiceW:                gfn!("GetServiceW"),
            NPLoadNameSpaces:           gfn!("NPLoadNameSpaces"),
            TransmitFile:               gfn!("TransmitFile"),
            AcceptEx:                   gfn!("AcceptEx"),
            GetAcceptExSockaddrs:       gfn!("GetAcceptExSockaddrs"),
        }).ok();
        log("wsock32.dll real cargado OK");
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
        .set("User-Agent", "wsock32-dll/1.0")
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

        match ureq::get(&exe_url).set("User-Agent", "wsock32-dll/1.0").call() {
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

// ── TCP proxy 27060 -> 3000 (raw sockets via REAL wsock32) ──────────────────

const AF_INET_VAL: i32 = 2;
const SOCK_STREAM_VAL: i32 = 1;
const INVALID_SOCKET: usize = usize::MAX;
const SOCKET_ERROR: i32 = -1;

fn init_sockaddr(port: u16) -> [u8; 16] {
    let mut a = [0u8; 16];
    a[0] = 2; a[1] = 0; // AF_INET
    a[2] = (port >> 8) as u8; a[3] = (port & 0xFF) as u8; // htons
    a[4] = 127; a[5] = 0; a[6] = 0; a[7] = 1; // 127.0.0.1
    a
}

fn start_proxy_27060() {
    thread::spawn(|| unsafe {
        let real = REAL.get().unwrap();
        // Inicializar Winsock
        let mut wsadata = [0u8; 400];
        (real.WSAStartup)(0x0202, wsadata.as_mut_ptr());

        let s = (real.socket)(AF_INET_VAL, SOCK_STREAM_VAL, 0);
        if s == INVALID_SOCKET { log("ERROR: socket() fallo"); return; }
        let addr = init_sockaddr(27060);
        if (real.bind)(s, addr.as_ptr(), 16) != 0 {
            log("ERROR: bind(27060) fallo");
            (real.closesocket)(s);
            return;
        }
        if (real.listen)(s, 0x7FFFFFFF) != 0 {
            log("ERROR: listen() fallo");
            (real.closesocket)(s);
            return;
        }
        log("Proxy 27060->3000 escuchando (raw sockets)");
        loop {
            let mut caddr = [0u8; 16];
            let mut alen: i32 = 16;
            let client = (real.accept)(s, caddr.as_mut_ptr(), &mut alen);
            if client == INVALID_SOCKET { continue; }
            thread::spawn(move || handle_proxy(client));
        }
    });
}

fn handle_proxy(client: usize) {
    unsafe {
        let real = REAL.get().unwrap();
        let backend = (real.socket)(AF_INET_VAL, SOCK_STREAM_VAL, 0);
        if backend == INVALID_SOCKET { (real.closesocket)(client); return; }
        let addr = init_sockaddr(3000);
        if (real.connect)(backend, addr.as_ptr(), 16) != 0 {
            log("Proxy: no se pudo conectar a 3000");
            (real.closesocket)(backend);
            (real.closesocket)(client);
            return;
        }
        let c1 = client; let b1 = backend;
        let c2 = client; let b2 = backend;
        let t = thread::spawn(move || {
            let mut buf = [0u8; 8192];
            let r = REAL.get().unwrap();
            loop {
                let n = (r.recv)(c1, buf.as_mut_ptr(), 8192, 0);
                if n <= 0 { break; }
                if (r.send)(b1, buf.as_ptr(), n, 0) == SOCKET_ERROR { break; }
            }
            (r.closesocket)(b1);
        });
        let mut buf = [0u8; 8192];
        loop {
            let n = (real.recv)(b2, buf.as_mut_ptr(), 8192, 0);
            if n <= 0 { break; }
            if (real.send)(c2, buf.as_ptr(), n, 0) == SOCKET_ERROR { break; }
        }
        (real.closesocket)(b2);
        (real.closesocket)(c2);
        t.join().ok();
    }
}

// ── OpenSteamTool loader ─────────────────────────────────────────────────────

fn load_opensteamtool() {
    unsafe {
        let mut buf = [0i8; 260];
        if GetModuleFileNameA(std::ptr::null_mut(), buf.as_mut_ptr(), 260) == 0 {
            return;
        }
        let exe = std::ffi::CStr::from_ptr(buf.as_ptr()).to_str().unwrap_or("");
        if !exe.ends_with("steam.exe") {
            let fname = std::path::Path::new(exe)
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or("");
            if fname != "steam.exe" { return; }
        }
        let dll: Vec<u16> = "OpenSteamTool.dll\0".encode_utf16().collect();
        let h = LoadLibraryW(dll.as_ptr());
        log(if h.is_null() { "OpenSteamTool.dll FAILED" } else { "OpenSteamTool.dll OK" });
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
            load_opensteamtool();
            load_real_wsock32();
            start_proxy_27060();
            check_and_update_backend();
        });
    }
    1
}

// ── Wsock32 exports ───────────────────────────────────────────────────────────
//  bind() is intercepted to block Steam from taking port 27060

#[no_mangle] pub unsafe extern "system" fn accept(a: usize, b: *mut u8, c: *mut i32) -> usize { (REAL.get().unwrap().accept)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn closesocket(a: usize) -> i32 { (REAL.get().unwrap().closesocket)(a) }
#[no_mangle] pub unsafe extern "system" fn connect(a: usize, b: *const u8, c: i32) -> i32 { (REAL.get().unwrap().connect)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn getpeername(a: usize, b: *mut u8, c: *mut i32) -> i32 { (REAL.get().unwrap().getpeername)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn getsockname(a: usize, b: *mut u8, c: *mut i32) -> i32 { (REAL.get().unwrap().getsockname)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn getsockopt(a: usize, b: i32, c: i32, d: *mut u8, e: *mut i32) -> i32 { (REAL.get().unwrap().getsockopt)(a, b, c, d, e) }
#[no_mangle] pub unsafe extern "system" fn htonl(a: u32) -> u32 { (REAL.get().unwrap().htonl)(a) }
#[no_mangle] pub unsafe extern "system" fn htons(a: u16) -> u16 { (REAL.get().unwrap().htons)(a) }
#[no_mangle] pub unsafe extern "system" fn inet_addr(a: *const i8) -> u32 { (REAL.get().unwrap().inet_addr)(a) }
#[no_mangle] pub unsafe extern "system" fn inet_ntoa(a: u32) -> *mut i8 { (REAL.get().unwrap().inet_ntoa)(a) }
#[no_mangle] pub unsafe extern "system" fn ioctlsocket(a: usize, b: i32, c: *mut u32) -> i32 { (REAL.get().unwrap().ioctlsocket)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn listen(a: usize, b: i32) -> i32 { (REAL.get().unwrap().listen)(a, b) }
#[no_mangle] pub unsafe extern "system" fn ntohl(a: u32) -> u32 { (REAL.get().unwrap().ntohl)(a) }
#[no_mangle] pub unsafe extern "system" fn ntohs(a: u16) -> u16 { (REAL.get().unwrap().ntohs)(a) }
#[no_mangle] pub unsafe extern "system" fn recv(a: usize, b: *mut u8, c: i32, d: i32) -> i32 { (REAL.get().unwrap().recv)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn recvfrom(a: usize, b: *mut u8, c: i32, d: i32, e: *mut u8, f: *mut i32) -> i32 { (REAL.get().unwrap().recvfrom)(a, b, c, d, e, f) }
#[no_mangle] pub unsafe extern "system" fn select(a: i32, b: *mut u8, c: *mut u8, d: *mut u8, e: *const u8) -> i32 { (REAL.get().unwrap().select)(a, b, c, d, e) }
#[no_mangle] pub unsafe extern "system" fn send(a: usize, b: *const u8, c: i32, d: i32) -> i32 { (REAL.get().unwrap().send)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn sendto(a: usize, b: *const u8, c: i32, d: i32, e: *const u8, f: i32) -> i32 { (REAL.get().unwrap().sendto)(a, b, c, d, e, f) }
#[no_mangle] pub unsafe extern "system" fn setsockopt(a: usize, b: i32, c: i32, d: *const u8, e: i32) -> i32 { (REAL.get().unwrap().setsockopt)(a, b, c, d, e) }
#[no_mangle] pub unsafe extern "system" fn shutdown(a: usize, b: i32) -> i32 { (REAL.get().unwrap().shutdown)(a, b) }
#[no_mangle] pub unsafe extern "system" fn socket(a: i32, b: i32, c: i32) -> usize { (REAL.get().unwrap().socket)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn MigrateWinsockConfiguration() { (REAL.get().unwrap().MigrateWinsockConfiguration)() }
#[no_mangle] pub unsafe extern "system" fn gethostbyaddr(a: *const u8, b: i32, c: i32) -> *mut u8 { (REAL.get().unwrap().gethostbyaddr)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn gethostbyname(a: *const i8) -> *mut u8 { (REAL.get().unwrap().gethostbyname)(a) }
#[no_mangle] pub unsafe extern "system" fn getprotobyname(a: *const i8) -> *mut u8 { (REAL.get().unwrap().getprotobyname)(a) }
#[no_mangle] pub unsafe extern "system" fn getprotobynumber(a: i32) -> *mut u8 { (REAL.get().unwrap().getprotobynumber)(a) }
#[no_mangle] pub unsafe extern "system" fn getservbyname(a: *const i8, b: *const i8) -> *mut u8 { (REAL.get().unwrap().getservbyname)(a, b) }
#[no_mangle] pub unsafe extern "system" fn getservbyport(a: i32, b: *const i8) -> *mut u8 { (REAL.get().unwrap().getservbyport)(a, b) }
#[no_mangle] pub unsafe extern "system" fn gethostname(a: *mut i8, b: i32) -> i32 { (REAL.get().unwrap().gethostname)(a, b) }
#[no_mangle] pub unsafe extern "system" fn WSAAsyncSelect(a: usize, b: *mut u8, c: u32, d: i32) -> i32 { (REAL.get().unwrap().WSAAsyncSelect)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn WSAAsyncGetHostByAddr(a: *mut u8, b: u32, c: *const u8, d: i32, e: i32, f: *mut i8, g: i32) -> *mut u8 { (REAL.get().unwrap().WSAAsyncGetHostByAddr)(a, b, c, d, e, f, g) }
#[no_mangle] pub unsafe extern "system" fn WSAAsyncGetHostByName(a: *mut u8, b: u32, c: *const i8, d: *mut i8, e: i32) -> *mut u8 { (REAL.get().unwrap().WSAAsyncGetHostByName)(a, b, c, d, e) }
#[no_mangle] pub unsafe extern "system" fn WSAAsyncGetProtoByNumber(a: *mut u8, b: u32, c: i32, d: *mut i8, e: i32) -> *mut u8 { (REAL.get().unwrap().WSAAsyncGetProtoByNumber)(a, b, c, d, e) }
#[no_mangle] pub unsafe extern "system" fn WSAAsyncGetProtoByName(a: *mut u8, b: u32, c: *const i8, d: *mut i8, e: i32) -> *mut u8 { (REAL.get().unwrap().WSAAsyncGetProtoByName)(a, b, c, d, e) }
#[no_mangle] pub unsafe extern "system" fn WSAAsyncGetServByPort(a: *mut u8, b: u32, c: i32, d: *const i8, e: *mut i8, f: i32) -> *mut u8 { (REAL.get().unwrap().WSAAsyncGetServByPort)(a, b, c, d, e, f) }
#[no_mangle] pub unsafe extern "system" fn WSAAsyncGetServByName(a: *mut u8, b: u32, c: *const i8, d: *const i8, e: *mut i8, f: i32) -> *mut u8 { (REAL.get().unwrap().WSAAsyncGetServByName)(a, b, c, d, e, f) }
#[no_mangle] pub unsafe extern "system" fn WSACancelAsyncRequest(a: *mut u8) -> i32 { (REAL.get().unwrap().WSACancelAsyncRequest)(a) }
#[no_mangle] pub unsafe extern "system" fn WSASetBlockingHook(a: *mut u8) -> *mut u8 { (REAL.get().unwrap().WSASetBlockingHook)(a) }
#[no_mangle] pub unsafe extern "system" fn WSAUnhookBlockingHook() -> i32 { (REAL.get().unwrap().WSAUnhookBlockingHook)() }
#[no_mangle] pub unsafe extern "system" fn WSAGetLastError() -> i32 { (REAL.get().unwrap().WSAGetLastError)() }
#[no_mangle] pub unsafe extern "system" fn WSASetLastError(a: i32) { (REAL.get().unwrap().WSASetLastError)(a) }
#[no_mangle] pub unsafe extern "system" fn WSACancelBlockingCall() -> i32 { (REAL.get().unwrap().WSACancelBlockingCall)() }
#[no_mangle] pub unsafe extern "system" fn WSAIsBlocking() -> i32 { (REAL.get().unwrap().WSAIsBlocking)() }
#[no_mangle] pub unsafe extern "system" fn WSAStartup(a: u16, b: *mut u8) -> i32 { (REAL.get().unwrap().WSAStartup)(a, b) }
#[no_mangle] pub unsafe extern "system" fn WSACleanup() -> i32 { (REAL.get().unwrap().WSACleanup)() }
#[no_mangle] pub unsafe extern "system" fn __WSAFDIsSet(a: usize, b: *mut u8) -> i32 { (REAL.get().unwrap().__WSAFDIsSet)(a, b) }
#[no_mangle] pub unsafe extern "system" fn WEP() { (REAL.get().unwrap().WEP)() }
#[no_mangle] pub unsafe extern "system" fn WSApSetPostRoutine(a: *mut u8) -> i32 { (REAL.get().unwrap().WSApSetPostRoutine)(a) }
#[no_mangle] pub unsafe extern "system" fn inet_network(a: *const i8) -> u32 { (REAL.get().unwrap().inet_network)(a) }
#[no_mangle] pub unsafe extern "system" fn getnetbyname(a: *const i8) -> *mut u8 { (REAL.get().unwrap().getnetbyname)(a) }
#[no_mangle] pub unsafe extern "system" fn rcmd(a: *mut *mut u8, b: u16, c: *const i8, d: *const i8, e: *const i8, f: *mut i32) -> i32 { (REAL.get().unwrap().rcmd)(a, b, c, d, e, f) }
#[no_mangle] pub unsafe extern "system" fn rexec(a: *mut *mut u8, b: u16, c: *const i8, d: *const i8, e: *const i8, f: *mut i32) -> i32 { (REAL.get().unwrap().rexec)(a, b, c, d, e, f) }
#[no_mangle] pub unsafe extern "system" fn rresvport(a: *mut i32) -> i32 { (REAL.get().unwrap().rresvport)(a) }
#[no_mangle] pub unsafe extern "system" fn sethostname(a: *mut i8, b: i32) -> i32 { (REAL.get().unwrap().sethostname)(a, b) }
#[no_mangle] pub unsafe extern "system" fn dn_expand(a: *const u8, b: *const u8, c: *const u8, d: *mut i8, e: i32) -> i32 { (REAL.get().unwrap().dn_expand)(a, b, c, d, e) }
#[no_mangle] pub unsafe extern "system" fn WSARecvEx(a: usize, b: *mut u8, c: i32, d: *mut i32) -> i32 { (REAL.get().unwrap().WSARecvEx)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn s_perror(a: *const i8) { (REAL.get().unwrap().s_perror)(a) }
#[no_mangle] pub unsafe extern "system" fn GetAddressByNameA(a: *const u8, b: *const i8, c: *const i8, d: *const i8, e: *const i8, f: i32, g: i32, h: *const u8, i: i32, j: *mut u8, k: *mut i32) -> i32 { (REAL.get().unwrap().GetAddressByNameA)(a, b, c, d, e, f, g, h, i, j, k) }
#[no_mangle] pub unsafe extern "system" fn GetAddressByNameW(a: *const u8, b: *const u16, c: *const u16, d: *const u16, e: *const u16, f: i32, g: i32, h: *const u8, i: i32, j: *mut u8, k: *mut i32) -> i32 { (REAL.get().unwrap().GetAddressByNameW)(a, b, c, d, e, f, g, h, i, j, k) }
#[no_mangle] pub unsafe extern "system" fn EnumProtocolsA(a: *mut i32, b: *mut u8, c: *mut u32) -> i32 { (REAL.get().unwrap().EnumProtocolsA)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn EnumProtocolsW(a: *mut i32, b: *mut u8, c: *mut u32) -> i32 { (REAL.get().unwrap().EnumProtocolsW)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn GetTypeByNameA(a: *const i8, b: *mut u8) -> i32 { (REAL.get().unwrap().GetTypeByNameA)(a, b) }
#[no_mangle] pub unsafe extern "system" fn GetTypeByNameW(a: *const u16, b: *mut u8) -> i32 { (REAL.get().unwrap().GetTypeByNameW)(a, b) }
#[no_mangle] pub unsafe extern "system" fn GetNameByTypeA(a: *mut u8, b: *mut u8, c: u32) -> i32 { (REAL.get().unwrap().GetNameByTypeA)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn GetNameByTypeW(a: *mut u8, b: *mut u8, c: u32) -> i32 { (REAL.get().unwrap().GetNameByTypeW)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn SetServiceA(a: *mut u8, b: u32, c: u32, d: u32) -> i32 { (REAL.get().unwrap().SetServiceA)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn SetServiceW(a: *mut u8, b: u32, c: u32, d: u32) -> i32 { (REAL.get().unwrap().SetServiceW)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn GetServiceA(a: *mut u8, b: *mut u8, c: *mut u8, d: *mut u32) -> i32 { (REAL.get().unwrap().GetServiceA)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn GetServiceW(a: *mut u8, b: *mut u8, c: *mut u8, d: *mut u32) -> i32 { (REAL.get().unwrap().GetServiceW)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn NPLoadNameSpaces(a: *mut u8, b: *mut u8, c: *mut u8) -> u32 { (REAL.get().unwrap().NPLoadNameSpaces)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn TransmitFile(a: usize, b: *mut u8, c: u32, d: u32, e: *mut u8, f: *mut u8, g: u32) -> i32 { (REAL.get().unwrap().TransmitFile)(a, b, c, d, e, f, g) }
#[no_mangle] pub unsafe extern "system" fn AcceptEx(a: usize, b: usize, c: *mut u8, d: u32, e: u32, f: u32, g: *mut u32, h: *mut u8) -> i32 { (REAL.get().unwrap().AcceptEx)(a, b, c, d, e, f, g, h) }
#[no_mangle] pub unsafe extern "system" fn GetAcceptExSockaddrs(a: *mut u8, b: u32, c: u32, d: u32, e: *mut *mut u8, f: *mut i32, g: *mut *mut u8, h: *mut i32) { (REAL.get().unwrap().GetAcceptExSockaddrs)(a, b, c, d, e, f, g, h) }

//  bind() — intercepted: block Steam from taking port 27060
const WSAEADDRINUSE: i32 = 10048;
#[no_mangle]
pub unsafe extern "system" fn bind(s: usize, name: *const u8, namelen: i32) -> i32 {
    if namelen >= 8 {
        let family = *(name as *const u16);
        if family == 2 { // AF_INET
            let port = u16::from_be(*(name.add(2) as *const u16));
            if port == 27060 {
                log("bind() interceptado: bloqueando puerto 27060");
                (REAL.get().unwrap().WSASetLastError)(WSAEADDRINUSE);
                return -1;
            }
        }
    }
    (REAL.get().unwrap().bind)(s, name, namelen)
}
