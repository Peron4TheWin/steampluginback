use std::thread;
use std::sync::OnceLock;
use std::io::Read;
use tiny_http::{Server, Response, Header};
//cache test upload
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

fn load_real_version() {
    unsafe {
        let lib = LoadLibraryA(b"C:\\Windows\\System32\\version.dll\0".as_ptr() as _);
        if lib.is_null() { return; }

        macro_rules! get_fn {
            ($name:ident) => {{
                let ptr = GetProcAddress(lib, concat!(stringify!($name), "\0").as_ptr() as _);
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
    }
}

#[no_mangle]
pub extern "system" fn DllMain(_hmodule: *mut u8, reason: u32, _reserved: *mut u8) -> i32 {
    if reason == 1 {
        load_real_version();
        init_api_key();
        thread::spawn(|| run_server());
    }
    1
}

use std::fs;
use std::path::Path;
use std::sync::{RwLock};

static API_KEY: OnceLock<RwLock<String>> = OnceLock::new();

fn init_api_key() {
    let path = "key.txt";

    if !Path::new(path).exists() {
        fs::write(path, "").expect("Failed to create key.txt");
    }

    let key = fs::read_to_string(path)
        .expect("Failed to read key.txt")
        .trim()
        .to_string();

    API_KEY.set(RwLock::new(key)).ok();
}

fn get_api_key() -> String {
    API_KEY
        .get()
        .unwrap()
        .read()
        .unwrap()
        .clone()
}

fn set_api_key(new_key: String) {
    {
        let mut key = API_KEY
            .get()
            .unwrap()
            .write()
            .unwrap();

        *key = new_key.clone();
    }

    fs::write("key.txt", new_key).expect("Failed to save key");
}
const LUA_DIR: &str = "C:\\Program Files (x86)\\Steam\\config\\stplug-in";

fn run_server() {
    let server = match Server::http("127.0.0.1:3000") {
        Ok(s) => s,
        Err(_) => return,
    };

    for mut request in server.incoming_requests() {
        if request.method().as_str() != "POST" {
            request.respond(Response::from_string("Method Not Allowed").with_status_code(405)).ok();
            continue;
        }
        if request.url().contains("key") {
            let mut body = String::new();
            if request.as_reader().read_to_string(&mut body).is_err() {
                request
                    .respond(Response::from_string("Failed to read body").with_status_code(400))
                    .ok();
                continue;
            }
            let key = body.trim();

            match ureq::get("https://hubcapmanifest.com/api/v1/user/stats")
                .set("Authorization", &format!("Bearer {}", key))
                .call()
            {
                Ok(_) => {
                    set_api_key(key.to_string());
                    request
                        .respond(Response::from_string("OK").with_status_code(200))
                        .ok();
                }
                Err(ureq::Error::Status(401, response)) => {
                    let msg = response
                        .into_string()
                        .unwrap_or_else(|_| "Unauthorized".to_string());
                    request
                        .respond(Response::from_string(msg).with_status_code(401))
                        .ok();
                }
                Err(err) => {
                    request
                        .respond(
                            Response::from_string(format!("API error: {}", err))
                                .with_status_code(500),
                        )
                        .ok();
                }
            }
            continue;
        }

        let appid = request.url().trim_start_matches('/').to_string();

        let mut _body = String::new();
        request.as_reader().read_to_string(&mut _body).ok();

        let url = format!("https://hubcapmanifest.com/api/v1/lua/{}", appid);
        let result = ureq::get(&url)
            .set("Authorization", &format!("Bearer {}", get_api_key()))
            .call();

        match result {
            Ok(resp) => {
                let mut buf = Vec::new();
                resp.into_reader().read_to_end(&mut buf).ok();
                std::fs::create_dir_all(LUA_DIR).ok();
                let path = format!("{}\\{}.lua", LUA_DIR, appid);
                std::fs::write(&path, &buf).ok();
                let response = Response::from_string("OK")
                    .with_header(
                        Header::from_bytes(
                            &b"Access-Control-Allow-Origin"[..],
                            &b"*"[..]
                        ).unwrap()
                    );

                request.respond(response).ok();
            }
            Err(ureq::Error::Status(code, resp)) => {
                let msg = resp.into_string().unwrap_or_default();
                let response = Response::from_string(msg)
                    .with_status_code(code)
                    .with_header(Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap());
                request.respond(response).ok();
            }
            Err(err) => {
                let response = Response::from_string(err.to_string())
                    .with_status_code(500)
                    .with_header(Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap());
                request.respond(response).ok();
            }
        }
    }
}


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
