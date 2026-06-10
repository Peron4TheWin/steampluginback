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

// ── Wsock32 proxy ─────────────────────────────────────────────────────────────

type SOCKET = usize;
type FARPROC = *mut u8;

type Fn_accept                   = unsafe extern "system" fn(SOCKET, *mut u8, *mut i32) -> SOCKET;
type Fn_bind                     = unsafe extern "system" fn(SOCKET, *const u8, i32) -> i32;
type Fn_closesocket              = unsafe extern "system" fn(SOCKET) -> i32;
type Fn_connect                  = unsafe extern "system" fn(SOCKET, *const u8, i32) -> i32;
type Fn_gethostbyaddr            = unsafe extern "system" fn(*const i8, i32, i32) -> *mut u8;
type Fn_gethostbyname            = unsafe extern "system" fn(*const i8) -> *mut u8;
type Fn_gethostname              = unsafe extern "system" fn(*mut i8, i32) -> i32;
type Fn_getpeername              = unsafe extern "system" fn(SOCKET, *mut u8, *mut i32) -> i32;
type Fn_getprotobyname           = unsafe extern "system" fn(*const i8) -> *mut u8;
type Fn_getprotobynumber         = unsafe extern "system" fn(i32) -> *mut u8;
type Fn_getservbyname            = unsafe extern "system" fn(*const i8, *const i8) -> *mut u8;
type Fn_getservbyport            = unsafe extern "system" fn(i32, *const i8) -> *mut u8;
type Fn_getsockname              = unsafe extern "system" fn(SOCKET, *mut u8, *mut i32) -> i32;
type Fn_getsockopt               = unsafe extern "system" fn(SOCKET, i32, i32, *mut i8, *mut i32) -> i32;
type Fn_htonl                    = unsafe extern "system" fn(u32) -> u32;
type Fn_htons                    = unsafe extern "system" fn(u16) -> u16;
type Fn_ioctlsocket              = unsafe extern "system" fn(SOCKET, i32, *mut u32) -> i32;
type Fn_listen                   = unsafe extern "system" fn(SOCKET, i32) -> i32;
type Fn_ntohl                    = unsafe extern "system" fn(u32) -> u32;
type Fn_ntohs                    = unsafe extern "system" fn(u16) -> u16;
type Fn_recv                     = unsafe extern "system" fn(SOCKET, *mut i8, i32, i32) -> i32;
type Fn_recvfrom                 = unsafe extern "system" fn(SOCKET, *mut i8, i32, i32, *mut u8, *mut i32) -> i32;
type Fn_select                   = unsafe extern "system" fn(i32, *mut u8, *mut u8, *mut u8, *const u8) -> i32;
type Fn_send                     = unsafe extern "system" fn(SOCKET, *const i8, i32, i32) -> i32;
type Fn_sendto                   = unsafe extern "system" fn(SOCKET, *const i8, i32, i32, *const u8, i32) -> i32;
type Fn_setsockopt               = unsafe extern "system" fn(SOCKET, i32, i32, *const i8, i32) -> i32;
type Fn_shutdown                 = unsafe extern "system" fn(SOCKET, i32) -> i32;
type Fn_socket                   = unsafe extern "system" fn(i32, i32, i32) -> SOCKET;
type Fn_WSAAsyncGetHostByAddr    = unsafe extern "system" fn(*mut u8, u32, *const i8, i32, i32, *mut i8, i32) -> *mut u8;
type Fn_WSAAsyncGetHostByName    = unsafe extern "system" fn(*mut u8, u32, *const i8, *mut i8, i32) -> *mut u8;
type Fn_WSAAsyncGetProtoByName   = unsafe extern "system" fn(*mut u8, u32, *const i8, *mut i8, i32) -> *mut u8;
type Fn_WSAAsyncGetProtoByNumber = unsafe extern "system" fn(*mut u8, u32, i32, *mut i8, i32) -> *mut u8;
type Fn_WSAAsyncGetServByName    = unsafe extern "system" fn(*mut u8, u32, *const i8, *const i8, *mut i8, i32) -> *mut u8;
type Fn_WSAAsyncGetServByPort    = unsafe extern "system" fn(*mut u8, u32, i32, *const i8, *mut i8, i32) -> *mut u8;
type Fn_WSAAsyncSelect           = unsafe extern "system" fn(SOCKET, *mut u8, u32, i32) -> i32;
type Fn_WSACancelAsyncRequest    = unsafe extern "system" fn(*mut u8) -> i32;
type Fn_WSACancelBlockingCall    = unsafe extern "system" fn() -> i32;
type Fn_WSACleanup               = unsafe extern "system" fn() -> i32;
type Fn_WSAGetLastError          = unsafe extern "system" fn() -> i32;
type Fn_WSAIsBlocking            = unsafe extern "system" fn() -> i32;
type Fn_WSASetBlockingHook       = unsafe extern "system" fn(FARPROC) -> FARPROC;
type Fn_WSASetLastError          = unsafe extern "system" fn(i32);
type Fn_WSAStartup               = unsafe extern "system" fn(u16, *mut u8) -> i32;
type Fn_WSAUnhookBlockingHook    = unsafe extern "system" fn() -> i32;
type Fn___WSAFdIsSet             = unsafe extern "system" fn(SOCKET, *mut u8) -> i32;

struct WsockFns {
    accept:                  Fn_accept,
    bind:                    Fn_bind,
    closesocket:             Fn_closesocket,
    connect:                 Fn_connect,
    gethostbyaddr:           Fn_gethostbyaddr,
    gethostbyname:           Fn_gethostbyname,
    gethostname:             Fn_gethostname,
    getpeername:             Fn_getpeername,
    getprotobyname:          Fn_getprotobyname,
    getprotobynumber:        Fn_getprotobynumber,
    getservbyname:           Fn_getservbyname,
    getservbyport:           Fn_getservbyport,
    getsockname:             Fn_getsockname,
    getsockopt:              Fn_getsockopt,
    htonl:                   Fn_htonl,
    htons:                   Fn_htons,
    ioctlsocket:             Fn_ioctlsocket,
    listen:                  Fn_listen,
    ntohl:                   Fn_ntohl,
    ntohs:                   Fn_ntohs,
    recv:                    Fn_recv,
    recvfrom:                Fn_recvfrom,
    select:                  Fn_select,
    send:                    Fn_send,
    sendto:                  Fn_sendto,
    setsockopt:              Fn_setsockopt,
    shutdown:                Fn_shutdown,
    socket:                  Fn_socket,
    WSAAsyncGetHostByAddr:   Fn_WSAAsyncGetHostByAddr,
    WSAAsyncGetHostByName:   Fn_WSAAsyncGetHostByName,
    WSAAsyncGetProtoByName:  Fn_WSAAsyncGetProtoByName,
    WSAAsyncGetProtoByNumber:Fn_WSAAsyncGetProtoByNumber,
    WSAAsyncGetServByName:   Fn_WSAAsyncGetServByName,
    WSAAsyncGetServByPort:   Fn_WSAAsyncGetServByPort,
    WSAAsyncSelect:          Fn_WSAAsyncSelect,
    WSACancelAsyncRequest:   Fn_WSACancelAsyncRequest,
    WSACancelBlockingCall:   Fn_WSACancelBlockingCall,
    WSACleanup:              Fn_WSACleanup,
    WSAGetLastError:         Fn_WSAGetLastError,
    WSAIsBlocking:           Fn_WSAIsBlocking,
    WSASetBlockingHook:      Fn_WSASetBlockingHook,
    WSASetLastError:         Fn_WSASetLastError,
    WSAStartup:              Fn_WSAStartup,
    WSAUnhookBlockingHook:   Fn_WSAUnhookBlockingHook,
    __WSAFdIsSet:            Fn___WSAFdIsSet,
}

unsafe impl Send for WsockFns {}
unsafe impl Sync for WsockFns {}

static REAL: OnceLock<WsockFns> = OnceLock::new();

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
            ($name:ident) => {{
                let ptr = GetProcAddress(lib, concat!(stringify!($name), "\0").as_ptr() as _);
                std::mem::transmute(ptr)
            }};
        }
        REAL.set(WsockFns {
            accept:                  gfn!(accept),
            bind:                    gfn!(bind),
            closesocket:             gfn!(closesocket),
            connect:                 gfn!(connect),
            gethostbyaddr:           gfn!(gethostbyaddr),
            gethostbyname:           gfn!(gethostbyname),
            gethostname:             gfn!(gethostname),
            getpeername:             gfn!(getpeername),
            getprotobyname:          gfn!(getprotobyname),
            getprotobynumber:        gfn!(getprotobynumber),
            getservbyname:           gfn!(getservbyname),
            getservbyport:           gfn!(getservbyport),
            getsockname:             gfn!(getsockname),
            getsockopt:              gfn!(getsockopt),
            htonl:                   gfn!(htonl),
            htons:                   gfn!(htons),
            ioctlsocket:             gfn!(ioctlsocket),
            listen:                  gfn!(listen),
            ntohl:                   gfn!(ntohl),
            ntohs:                   gfn!(ntohs),
            recv:                    gfn!(recv),
            recvfrom:                gfn!(recvfrom),
            select:                  gfn!(select),
            send:                    gfn!(send),
            sendto:                  gfn!(sendto),
            setsockopt:              gfn!(setsockopt),
            shutdown:                gfn!(shutdown),
            socket:                  gfn!(socket),
            WSAAsyncGetHostByAddr:   gfn!(WSAAsyncGetHostByAddr),
            WSAAsyncGetHostByName:   gfn!(WSAAsyncGetHostByName),
            WSAAsyncGetProtoByName:  gfn!(WSAAsyncGetProtoByName),
            WSAAsyncGetProtoByNumber:gfn!(WSAAsyncGetProtoByNumber),
            WSAAsyncGetServByName:   gfn!(WSAAsyncGetServByName),
            WSAAsyncGetServByPort:   gfn!(WSAAsyncGetServByPort),
            WSAAsyncSelect:          gfn!(WSAAsyncSelect),
            WSACancelAsyncRequest:   gfn!(WSACancelAsyncRequest),
            WSACancelBlockingCall:   gfn!(WSACancelBlockingCall),
            WSACleanup:              gfn!(WSACleanup),
            WSAGetLastError:         gfn!(WSAGetLastError),
            WSAIsBlocking:           gfn!(WSAIsBlocking),
            WSASetBlockingHook:      gfn!(WSASetBlockingHook),
            WSASetLastError:         gfn!(WSASetLastError),
            WSAStartup:              gfn!(WSAStartup),
            WSAUnhookBlockingHook:   gfn!(WSAUnhookBlockingHook),
            __WSAFdIsSet:            gfn!(__WSAFdIsSet),
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
    let result = hasher.finalize();
    Some(format!("{:x}", result))
}

// ── GitHub release check + download ──────────────────────────────────────────

fn extract_str_after<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{}\":", key);
    let start = json.find(&needle)? + needle.len();
    let rest = json[start..].trim_start();
    if !rest.starts_with('"') { return None; }
    let inner = &rest[1..];
    let end = inner.find('"')?;
    Some(&inner[..end])
}

fn find_exe_asset(json: &str) -> Option<(String, String)> {
    let mut search = json;
    while let Some(pos) = search.find("\"name\":") {
        let rest = &search[pos..];
        if let Some(name) = extract_str_after(rest, "name") {
            if name.ends_with(".exe") {
                let url = extract_str_after(rest, "browser_download_url")
                    .unwrap_or("")
                    .to_string();
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
        .set("User-Agent", "wsock32-dll/1.0")
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
            // Si falla la API igual intentamos lanzar lo que haya
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

        // Primero matamos el proceso que lo tiene bloqueado
        kill_port_3000();
        std::thread::sleep(std::time::Duration::from_millis(1500));

        // Ahora sí borramos
        if Path::new(&local_path).exists() {
            if let Err(e) = fs::remove_file(&local_path) {
                log(&format!("ERROR borrando backend.exe viejo: {}", e));
                return;
            }
            log("backend.exe viejo borrado");
        }

        // Descargamos
        match ureq::get(&exe_url)
            .set("User-Agent", "wsock32-dll/1.0")
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

        // Lanzamos después de actualizar
        launch_backend();
        return;
    }

    // Lanzamos (caso sha coincide, proceso puede estar caído)
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
        let ret = GetExtendedTcpTable(buf.as_mut_ptr(), &mut size, 0, AF_INET, TCP_TABLE_OWNER_PID_ALL, 0);
        if ret != 0 { return; }

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
    if !Path::new(&path).exists() {
        log("backend.exe no existe, no se puede lanzar");
        return;
    }

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let mut si = vec![0u8; 104];
    si[0] = 104;
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
            load_real_wsock32();
            check_and_update_backend();
        });
    }
    1
}

// ── Wsock32 exports ───────────────────────────────────────────────────────────

#[no_mangle] pub unsafe extern "system" fn accept(s: SOCKET, addr: *mut u8, addrlen: *mut i32) -> SOCKET { (REAL.get().unwrap().accept)(s, addr, addrlen) }
#[no_mangle] pub unsafe extern "system" fn bind(s: SOCKET, name: *const u8, namelen: i32) -> i32 { (REAL.get().unwrap().bind)(s, name, namelen) }
#[no_mangle] pub unsafe extern "system" fn closesocket(s: SOCKET) -> i32 { (REAL.get().unwrap().closesocket)(s) }
#[no_mangle] pub unsafe extern "system" fn connect(s: SOCKET, name: *const u8, namelen: i32) -> i32 { (REAL.get().unwrap().connect)(s, name, namelen) }
#[no_mangle] pub unsafe extern "system" fn gethostbyaddr(addr: *const i8, len: i32, r#type: i32) -> *mut u8 { (REAL.get().unwrap().gethostbyaddr)(addr, len, r#type) }
#[no_mangle] pub unsafe extern "system" fn gethostbyname(name: *const i8) -> *mut u8 { (REAL.get().unwrap().gethostbyname)(name) }
#[no_mangle] pub unsafe extern "system" fn gethostname(name: *mut i8, namelen: i32) -> i32 { (REAL.get().unwrap().gethostname)(name, namelen) }
#[no_mangle] pub unsafe extern "system" fn getpeername(s: SOCKET, name: *mut u8, namelen: *mut i32) -> i32 { (REAL.get().unwrap().getpeername)(s, name, namelen) }
#[no_mangle] pub unsafe extern "system" fn getprotobyname(name: *const i8) -> *mut u8 { (REAL.get().unwrap().getprotobyname)(name) }
#[no_mangle] pub unsafe extern "system" fn getprotobynumber(number: i32) -> *mut u8 { (REAL.get().unwrap().getprotobynumber)(number) }
#[no_mangle] pub unsafe extern "system" fn getservbyname(name: *const i8, proto: *const i8) -> *mut u8 { (REAL.get().unwrap().getservbyname)(name, proto) }
#[no_mangle] pub unsafe extern "system" fn getservbyport(port: i32, proto: *const i8) -> *mut u8 { (REAL.get().unwrap().getservbyport)(port, proto) }
#[no_mangle] pub unsafe extern "system" fn getsockname(s: SOCKET, name: *mut u8, namelen: *mut i32) -> i32 { (REAL.get().unwrap().getsockname)(s, name, namelen) }
#[no_mangle] pub unsafe extern "system" fn getsockopt(s: SOCKET, level: i32, optname: i32, optval: *mut i8, optlen: *mut i32) -> i32 { (REAL.get().unwrap().getsockopt)(s, level, optname, optval, optlen) }
#[no_mangle] pub unsafe extern "system" fn htonl(hostlong: u32) -> u32 { (REAL.get().unwrap().htonl)(hostlong) }
#[no_mangle] pub unsafe extern "system" fn htons(hostshort: u16) -> u16 { (REAL.get().unwrap().htons)(hostshort) }
#[no_mangle] pub unsafe extern "system" fn ioctlsocket(s: SOCKET, cmd: i32, argp: *mut u32) -> i32 { (REAL.get().unwrap().ioctlsocket)(s, cmd, argp) }
#[no_mangle] pub unsafe extern "system" fn listen(s: SOCKET, backlog: i32) -> i32 { (REAL.get().unwrap().listen)(s, backlog) }
#[no_mangle] pub unsafe extern "system" fn ntohl(netlong: u32) -> u32 { (REAL.get().unwrap().ntohl)(netlong) }
#[no_mangle] pub unsafe extern "system" fn ntohs(netshort: u16) -> u16 { (REAL.get().unwrap().ntohs)(netshort) }
#[no_mangle] pub unsafe extern "system" fn recv(s: SOCKET, buf: *mut i8, len: i32, flags: i32) -> i32 { (REAL.get().unwrap().recv)(s, buf, len, flags) }
#[no_mangle] pub unsafe extern "system" fn recvfrom(s: SOCKET, buf: *mut i8, len: i32, flags: i32, from: *mut u8, fromlen: *mut i32) -> i32 { (REAL.get().unwrap().recvfrom)(s, buf, len, flags, from, fromlen) }
#[no_mangle] pub unsafe extern "system" fn select(nfds: i32, readfds: *mut u8, writefds: *mut u8, exceptfds: *mut u8, timeout: *const u8) -> i32 { (REAL.get().unwrap().select)(nfds, readfds, writefds, exceptfds, timeout) }
#[no_mangle] pub unsafe extern "system" fn send(s: SOCKET, buf: *const i8, len: i32, flags: i32) -> i32 { (REAL.get().unwrap().send)(s, buf, len, flags) }
#[no_mangle] pub unsafe extern "system" fn sendto(s: SOCKET, buf: *const i8, len: i32, flags: i32, to: *const u8, tolen: i32) -> i32 { (REAL.get().unwrap().sendto)(s, buf, len, flags, to, tolen) }
#[no_mangle] pub unsafe extern "system" fn setsockopt(s: SOCKET, level: i32, optname: i32, optval: *const i8, optlen: i32) -> i32 { (REAL.get().unwrap().setsockopt)(s, level, optname, optval, optlen) }
#[no_mangle] pub unsafe extern "system" fn shutdown(s: SOCKET, how: i32) -> i32 { (REAL.get().unwrap().shutdown)(s, how) }
#[no_mangle] pub unsafe extern "system" fn socket(af: i32, r#type: i32, protocol: i32) -> SOCKET { (REAL.get().unwrap().socket)(af, r#type, protocol) }
#[no_mangle] pub unsafe extern "system" fn WSAAsyncGetHostByAddr(hwnd: *mut u8, wmsg: u32, addr: *const i8, len: i32, r#type: i32, buf: *mut i8, buflen: i32) -> *mut u8 { (REAL.get().unwrap().WSAAsyncGetHostByAddr)(hwnd, wmsg, addr, len, r#type, buf, buflen) }
#[no_mangle] pub unsafe extern "system" fn WSAAsyncGetHostByName(hwnd: *mut u8, wmsg: u32, name: *const i8, buf: *mut i8, buflen: i32) -> *mut u8 { (REAL.get().unwrap().WSAAsyncGetHostByName)(hwnd, wmsg, name, buf, buflen) }
#[no_mangle] pub unsafe extern "system" fn WSAAsyncGetProtoByName(hwnd: *mut u8, wmsg: u32, name: *const i8, buf: *mut i8, buflen: i32) -> *mut u8 { (REAL.get().unwrap().WSAAsyncGetProtoByName)(hwnd, wmsg, name, buf, buflen) }
#[no_mangle] pub unsafe extern "system" fn WSAAsyncGetProtoByNumber(hwnd: *mut u8, wmsg: u32, number: i32, buf: *mut i8, buflen: i32) -> *mut u8 { (REAL.get().unwrap().WSAAsyncGetProtoByNumber)(hwnd, wmsg, number, buf, buflen) }
#[no_mangle] pub unsafe extern "system" fn WSAAsyncGetServByName(hwnd: *mut u8, wmsg: u32, name: *const i8, proto: *const i8, buf: *mut i8, buflen: i32) -> *mut u8 { (REAL.get().unwrap().WSAAsyncGetServByName)(hwnd, wmsg, name, proto, buf, buflen) }
#[no_mangle] pub unsafe extern "system" fn WSAAsyncGetServByPort(hwnd: *mut u8, wmsg: u32, port: i32, proto: *const i8, buf: *mut i8, buflen: i32) -> *mut u8 { (REAL.get().unwrap().WSAAsyncGetServByPort)(hwnd, wmsg, port, proto, buf, buflen) }
#[no_mangle] pub unsafe extern "system" fn WSAAsyncSelect(s: SOCKET, hwnd: *mut u8, wmsg: u32, levent: i32) -> i32 { (REAL.get().unwrap().WSAAsyncSelect)(s, hwnd, wmsg, levent) }
#[no_mangle] pub unsafe extern "system" fn WSACancelAsyncRequest(hasynctaskhandle: *mut u8) -> i32 { (REAL.get().unwrap().WSACancelAsyncRequest)(hasynctaskhandle) }
#[no_mangle] pub unsafe extern "system" fn WSACancelBlockingCall() -> i32 { (REAL.get().unwrap().WSACancelBlockingCall)() }
#[no_mangle] pub unsafe extern "system" fn WSACleanup() -> i32 { (REAL.get().unwrap().WSACleanup)() }
#[no_mangle] pub unsafe extern "system" fn WSAGetLastError() -> i32 { (REAL.get().unwrap().WSAGetLastError)() }
#[no_mangle] pub unsafe extern "system" fn WSAIsBlocking() -> i32 { (REAL.get().unwrap().WSAIsBlocking)() }
#[no_mangle] pub unsafe extern "system" fn WSASetBlockingHook(lpblockfunc: FARPROC) -> FARPROC { (REAL.get().unwrap().WSASetBlockingHook)(lpblockfunc) }
#[no_mangle] pub unsafe extern "system" fn WSASetLastError(ierror: i32) { (REAL.get().unwrap().WSASetLastError)(ierror) }
#[no_mangle] pub unsafe extern "system" fn WSAStartup(wversionrequested: u16, lpwsadata: *mut u8) -> i32 { (REAL.get().unwrap().WSAStartup)(wversionrequested, lpwsadata) }
#[no_mangle] pub unsafe extern "system" fn WSAUnhookBlockingHook() -> i32 { (REAL.get().unwrap().WSAUnhookBlockingHook)() }
#[no_mangle] pub unsafe extern "system" fn __WSAFdIsSet(s: SOCKET, set: *mut u8) -> i32 { (REAL.get().unwrap().__WSAFdIsSet)(s, set) }