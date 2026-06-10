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
fn log_path() -> String { format!("{}\\winmm_dll.log", steam_dir()) }
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

// ── Winmm proxy ───────────────────────────────────────────────────────────────

type Fn_CloseDriver              = unsafe extern "system" fn(*mut u8, usize, isize) -> isize;
type Fn_DefDriverProc            = unsafe extern "system" fn(usize, *mut u8, u32, isize, isize) -> isize;
type Fn_DriverCallback           = unsafe extern "system" fn(usize, u32, *mut u8, usize, usize) -> i32;
type Fn_DrvGetModuleHandle       = unsafe extern "system" fn(*mut u8) -> *mut u8;
type Fn_GetDriverModuleHandle    = unsafe extern "system" fn(*mut u8) -> *mut u8;
type Fn_NotifyCallbackData       = unsafe extern "system" fn(usize, u32, usize, usize, usize) -> u32;
type Fn_OpenDriver               = unsafe extern "system" fn(*const u16, *const u16, isize) -> *mut u8;
type Fn_PlaySoundA               = unsafe extern "system" fn(*const i8, *mut u8, u32) -> i32;
type Fn_PlaySoundW               = unsafe extern "system" fn(*const u16, *mut u8, u32) -> i32;
type Fn_SendDriverMessage        = unsafe extern "system" fn(*mut u8, u32, isize, isize) -> isize;
type Fn_auxGetDevCapsA           = unsafe extern "system" fn(usize, *mut u8, u32) -> u32;
type Fn_auxGetDevCapsW           = unsafe extern "system" fn(usize, *mut u8, u32) -> u32;
type Fn_auxGetNumDevs            = unsafe extern "system" fn() -> u32;
type Fn_auxGetVolume             = unsafe extern "system" fn(u32, *mut u32) -> u32;
type Fn_auxOutMessage            = unsafe extern "system" fn(u32, u32, usize, usize) -> u32;
type Fn_auxSetVolume             = unsafe extern "system" fn(u32, u32) -> u32;
type Fn_joyConfigChanged         = unsafe extern "system" fn(u32) -> u32;
type Fn_joyGetDevCapsA           = unsafe extern "system" fn(usize, *mut u8, u32) -> u32;
type Fn_joyGetDevCapsW           = unsafe extern "system" fn(usize, *mut u8, u32) -> u32;
type Fn_joyGetNumDevs            = unsafe extern "system" fn() -> u32;
type Fn_joyGetPos                = unsafe extern "system" fn(u32, *mut u8) -> u32;
type Fn_joyGetPosEx              = unsafe extern "system" fn(u32, *mut u8) -> u32;
type Fn_joyGetThreshold          = unsafe extern "system" fn(u32, *mut u32) -> u32;
type Fn_joyReleaseCapture        = unsafe extern "system" fn(u32) -> u32;
type Fn_joySetCapture            = unsafe extern "system" fn(*mut u8, u32, u32, i32) -> u32;
type Fn_joySetThreshold          = unsafe extern "system" fn(u32, u32) -> u32;
type Fn_mciDriverNotify          = unsafe extern "system" fn(*mut u8, u32, u32) -> i32;
type Fn_mciDriverYield           = unsafe extern "system" fn(u32) -> u32;
type Fn_mciExecute               = unsafe extern "system" fn(*const i8) -> i32;
type Fn_mciFreeCommandResource   = unsafe extern "system" fn(u32) -> i32;
type Fn_mciGetCreatorTask        = unsafe extern "system" fn(u32) -> *mut u8;
type Fn_mciGetDeviceIDA          = unsafe extern "system" fn(*const i8) -> u32;
type Fn_mciGetDeviceIDFromElementIDA = unsafe extern "system" fn(u32, *const i8) -> u32;
type Fn_mciGetDeviceIDFromElementIDW = unsafe extern "system" fn(u32, *const u16) -> u32;
type Fn_mciGetDeviceIDW          = unsafe extern "system" fn(*const u16) -> u32;
type Fn_mciGetDriverData         = unsafe extern "system" fn(u32) -> usize;
type Fn_mciGetErrorStringA       = unsafe extern "system" fn(u32, *mut i8, u32) -> i32;
type Fn_mciGetErrorStringW       = unsafe extern "system" fn(u32, *mut u16, u32) -> i32;
type Fn_mciGetYieldProc          = unsafe extern "system" fn(u32, *mut u32) -> *mut u8;
type Fn_mciLoadCommandResource   = unsafe extern "system" fn(*mut u8, *const u16, u32) -> u32;
type Fn_mciSendCommandA          = unsafe extern "system" fn(u32, u32, usize, usize) -> u32;
type Fn_mciSendCommandW          = unsafe extern "system" fn(u32, u32, usize, usize) -> u32;
type Fn_mciSendStringA           = unsafe extern "system" fn(*const i8, *mut i8, u32, *mut u8) -> u32;
type Fn_mciSendStringW           = unsafe extern "system" fn(*const u16, *mut u16, u32, *mut u8) -> u32;
type Fn_mciSetDriverData         = unsafe extern "system" fn(u32, usize) -> i32;
type Fn_mciSetYieldProc          = unsafe extern "system" fn(u32, *mut u8, u32) -> i32;
type Fn_midiConnect              = unsafe extern "system" fn(*mut u8, *mut u8, *mut u8) -> u32;
type Fn_midiDisconnect           = unsafe extern "system" fn(*mut u8, *mut u8, *mut u8) -> u32;
type Fn_midiInAddBuffer          = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_midiInClose              = unsafe extern "system" fn(*mut u8) -> u32;
type Fn_midiInGetDevCapsA        = unsafe extern "system" fn(usize, *mut u8, u32) -> u32;
type Fn_midiInGetDevCapsW        = unsafe extern "system" fn(usize, *mut u8, u32) -> u32;
type Fn_midiInGetErrorTextA      = unsafe extern "system" fn(u32, *mut i8, u32) -> u32;
type Fn_midiInGetErrorTextW      = unsafe extern "system" fn(u32, *mut u16, u32) -> u32;
type Fn_midiInGetID              = unsafe extern "system" fn(*mut u8, *mut u32) -> u32;
type Fn_midiInGetNumDevs         = unsafe extern "system" fn() -> u32;
type Fn_midiInMessage            = unsafe extern "system" fn(*mut u8, u32, usize, usize) -> u32;
type Fn_midiInOpen               = unsafe extern "system" fn(*mut *mut u8, u32, usize, usize, u32) -> u32;
type Fn_midiInPrepareHeader      = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_midiInReset              = unsafe extern "system" fn(*mut u8) -> u32;
type Fn_midiInStart              = unsafe extern "system" fn(*mut u8) -> u32;
type Fn_midiInStop               = unsafe extern "system" fn(*mut u8) -> u32;
type Fn_midiInUnprepareHeader    = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_midiOutCacheDrumPatches  = unsafe extern "system" fn(*mut u8, u32, *mut u8, u32) -> u32;
type Fn_midiOutCachePatches      = unsafe extern "system" fn(*mut u8, u32, *mut u8, u32) -> u32;
type Fn_midiOutClose             = unsafe extern "system" fn(*mut u8) -> u32;
type Fn_midiOutGetDevCapsA       = unsafe extern "system" fn(usize, *mut u8, u32) -> u32;
type Fn_midiOutGetDevCapsW       = unsafe extern "system" fn(usize, *mut u8, u32) -> u32;
type Fn_midiOutGetErrorTextA     = unsafe extern "system" fn(u32, *mut i8, u32) -> u32;
type Fn_midiOutGetErrorTextW     = unsafe extern "system" fn(u32, *mut u16, u32) -> u32;
type Fn_midiOutGetID             = unsafe extern "system" fn(*mut u8, *mut u32) -> u32;
type Fn_midiOutGetNumDevs        = unsafe extern "system" fn() -> u32;
type Fn_midiOutGetVolume         = unsafe extern "system" fn(*mut u8, *mut u32) -> u32;
type Fn_midiOutLongMsg           = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_midiOutMessage           = unsafe extern "system" fn(*mut u8, u32, usize, usize) -> u32;
type Fn_midiOutOpen              = unsafe extern "system" fn(*mut *mut u8, u32, usize, usize, u32) -> u32;
type Fn_midiOutPrepareHeader     = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_midiOutReset             = unsafe extern "system" fn(*mut u8) -> u32;
type Fn_midiOutSetVolume         = unsafe extern "system" fn(*mut u8, u32) -> u32;
type Fn_midiOutShortMsg          = unsafe extern "system" fn(*mut u8, u32) -> u32;
type Fn_midiOutUnprepareHeader   = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_midiStreamClose          = unsafe extern "system" fn(*mut u8) -> u32;
type Fn_midiStreamOpen           = unsafe extern "system" fn(*mut *mut u8, *mut u32, u32, usize, usize, u32) -> u32;
type Fn_midiStreamOut            = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_midiStreamPause          = unsafe extern "system" fn(*mut u8) -> u32;
type Fn_midiStreamPosition       = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_midiStreamProperty       = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_midiStreamRestart        = unsafe extern "system" fn(*mut u8) -> u32;
type Fn_midiStreamStop           = unsafe extern "system" fn(*mut u8) -> u32;
type Fn_mixerClose               = unsafe extern "system" fn(*mut u8) -> u32;
type Fn_mixerGetControlDetailsA  = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_mixerGetControlDetailsW  = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_mixerGetDevCapsA         = unsafe extern "system" fn(usize, *mut u8, u32) -> u32;
type Fn_mixerGetDevCapsW         = unsafe extern "system" fn(usize, *mut u8, u32) -> u32;
type Fn_mixerGetID               = unsafe extern "system" fn(*mut u8, *mut u32, u32) -> u32;
type Fn_mixerGetLineControlsA    = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_mixerGetLineControlsW    = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_mixerGetLineInfoA        = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_mixerGetLineInfoW        = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_mixerGetNumDevs          = unsafe extern "system" fn() -> u32;
type Fn_mixerMessage             = unsafe extern "system" fn(*mut u8, u32, usize, usize) -> u32;
type Fn_mixerOpen                = unsafe extern "system" fn(*mut *mut u8, u32, usize, usize, u32) -> u32;
type Fn_mixerSetControlDetails   = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_mmGetCurrentTask         = unsafe extern "system" fn() -> *mut u8;
type Fn_mmTaskBlock              = unsafe extern "system" fn(u32);
type Fn_mmTaskCreate             = unsafe extern "system" fn(*mut u8, *mut *mut u8, usize) -> u32;
type Fn_mmTaskSignal             = unsafe extern "system" fn(u32) -> i32;
type Fn_mmTaskYield              = unsafe extern "system" fn();
type Fn_mmioAdvance              = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_mmioAscend               = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_mmioClose                = unsafe extern "system" fn(*mut u8, u32) -> u32;
type Fn_mmioCreateChunk          = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_mmioDescend              = unsafe extern "system" fn(*mut u8, *mut u8, *mut u8, u32) -> u32;
type Fn_mmioFlush                = unsafe extern "system" fn(*mut u8, u32) -> u32;
type Fn_mmioGetInfo              = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_mmioInstallIOProcA       = unsafe extern "system" fn(*const i8, *mut u8, u32) -> *mut u8;
type Fn_mmioInstallIOProcW       = unsafe extern "system" fn(*const u16, *mut u8, u32) -> *mut u8;
type Fn_mmioOpenA                = unsafe extern "system" fn(*mut i8, *mut u8, u32) -> *mut u8;
type Fn_mmioOpenW                = unsafe extern "system" fn(*mut u16, *mut u8, u32) -> *mut u8;
type Fn_mmioRead                 = unsafe extern "system" fn(*mut u8, *mut i8, i32) -> i32;
type Fn_mmioRenameA              = unsafe extern "system" fn(*const i8, *const i8, *mut u8, u32) -> u32;
type Fn_mmioRenameW              = unsafe extern "system" fn(*const u16, *const u16, *mut u8, u32) -> u32;
type Fn_mmioSeek                 = unsafe extern "system" fn(*mut u8, i32, i32) -> i32;
type Fn_mmioSendMessage          = unsafe extern "system" fn(*mut u8, u32, isize, isize) -> isize;
type Fn_mmioSetInfo              = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_mmioStringToFOURCCA      = unsafe extern "system" fn(*const i8, u32) -> u32;
type Fn_mmioStringToFOURCCW      = unsafe extern "system" fn(*const u16, u32) -> u32;
type Fn_mmioWrite                = unsafe extern "system" fn(*mut u8, *const i8, i32) -> i32;
type Fn_mmsystemGetVersion       = unsafe extern "system" fn() -> u32;
type Fn_sndPlaySoundA            = unsafe extern "system" fn(*const i8, u32) -> i32;
type Fn_sndPlaySoundW            = unsafe extern "system" fn(*const u16, u32) -> i32;
type Fn_timeBeginPeriod          = unsafe extern "system" fn(u32) -> u32;
type Fn_timeEndPeriod            = unsafe extern "system" fn(u32) -> u32;
type Fn_timeGetDevCaps           = unsafe extern "system" fn(*mut u8, u32) -> u32;
type Fn_timeGetSystemTime        = unsafe extern "system" fn(*mut u8, u32) -> u32;
type Fn_timeGetTime              = unsafe extern "system" fn() -> u32;
type Fn_timeKillEvent            = unsafe extern "system" fn(u32) -> u32;
type Fn_timeSetEvent             = unsafe extern "system" fn(u32, u32, *mut u8, usize, u32) -> u32;
type Fn_waveInAddBuffer          = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_waveInClose              = unsafe extern "system" fn(*mut u8) -> u32;
type Fn_waveInGetDevCapsA        = unsafe extern "system" fn(usize, *mut u8, u32) -> u32;
type Fn_waveInGetDevCapsW        = unsafe extern "system" fn(usize, *mut u8, u32) -> u32;
type Fn_waveInGetErrorTextA      = unsafe extern "system" fn(u32, *mut i8, u32) -> u32;
type Fn_waveInGetErrorTextW      = unsafe extern "system" fn(u32, *mut u16, u32) -> u32;
type Fn_waveInGetID              = unsafe extern "system" fn(*mut u8, *mut u32) -> u32;
type Fn_waveInGetNumDevs         = unsafe extern "system" fn() -> u32;
type Fn_waveInGetPosition        = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_waveInMessage            = unsafe extern "system" fn(*mut u8, u32, usize, usize) -> u32;
type Fn_waveInOpen               = unsafe extern "system" fn(*mut *mut u8, u32, *mut u8, usize, usize, u32) -> u32;
type Fn_waveInPrepareHeader      = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_waveInReset              = unsafe extern "system" fn(*mut u8) -> u32;
type Fn_waveInStart              = unsafe extern "system" fn(*mut u8) -> u32;
type Fn_waveInStop               = unsafe extern "system" fn(*mut u8) -> u32;
type Fn_waveInUnprepareHeader    = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_waveOutBreakLoop         = unsafe extern "system" fn(*mut u8) -> u32;
type Fn_waveOutClose             = unsafe extern "system" fn(*mut u8) -> u32;
type Fn_waveOutGetDevCapsA       = unsafe extern "system" fn(usize, *mut u8, u32) -> u32;
type Fn_waveOutGetDevCapsW       = unsafe extern "system" fn(usize, *mut u8, u32) -> u32;
type Fn_waveOutGetErrorTextA     = unsafe extern "system" fn(u32, *mut i8, u32) -> u32;
type Fn_waveOutGetErrorTextW     = unsafe extern "system" fn(u32, *mut u16, u32) -> u32;
type Fn_waveOutGetID             = unsafe extern "system" fn(*mut u8, *mut u32) -> u32;
type Fn_waveOutGetNumDevs        = unsafe extern "system" fn() -> u32;
type Fn_waveOutGetPitch          = unsafe extern "system" fn(*mut u8, *mut u32) -> u32;
type Fn_waveOutGetPlaybackRate   = unsafe extern "system" fn(*mut u8, *mut u32) -> u32;
type Fn_waveOutGetPosition       = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_waveOutGetVolume         = unsafe extern "system" fn(*mut u8, *mut u32) -> u32;
type Fn_waveOutMessage           = unsafe extern "system" fn(*mut u8, u32, usize, usize) -> u32;
type Fn_waveOutOpen              = unsafe extern "system" fn(*mut *mut u8, u32, *mut u8, usize, usize, u32) -> u32;
type Fn_waveOutPause             = unsafe extern "system" fn(*mut u8) -> u32;
type Fn_waveOutPrepareHeader     = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_waveOutReset             = unsafe extern "system" fn(*mut u8) -> u32;
type Fn_waveOutRestart           = unsafe extern "system" fn(*mut u8) -> u32;
type Fn_waveOutSetPitch          = unsafe extern "system" fn(*mut u8, u32) -> u32;
type Fn_waveOutSetPlaybackRate   = unsafe extern "system" fn(*mut u8, u32) -> u32;
type Fn_waveOutSetVolume         = unsafe extern "system" fn(*mut u8, u32) -> u32;
type Fn_waveOutUnprepareHeader   = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;
type Fn_waveOutWrite             = unsafe extern "system" fn(*mut u8, *mut u8, u32) -> u32;

struct WinmmFns {
    CloseDriver:                  Fn_CloseDriver,
    DefDriverProc:                Fn_DefDriverProc,
    DriverCallback:               Fn_DriverCallback,
    DrvGetModuleHandle:           Fn_DrvGetModuleHandle,
    GetDriverModuleHandle:        Fn_GetDriverModuleHandle,
    NotifyCallbackData:           Fn_NotifyCallbackData,
    OpenDriver:                   Fn_OpenDriver,
    PlaySoundA:                   Fn_PlaySoundA,
    PlaySoundW:                   Fn_PlaySoundW,
    SendDriverMessage:            Fn_SendDriverMessage,
    auxGetDevCapsA:               Fn_auxGetDevCapsA,
    auxGetDevCapsW:               Fn_auxGetDevCapsW,
    auxGetNumDevs:                Fn_auxGetNumDevs,
    auxGetVolume:                 Fn_auxGetVolume,
    auxOutMessage:                Fn_auxOutMessage,
    auxSetVolume:                 Fn_auxSetVolume,
    joyConfigChanged:             Fn_joyConfigChanged,
    joyGetDevCapsA:               Fn_joyGetDevCapsA,
    joyGetDevCapsW:               Fn_joyGetDevCapsW,
    joyGetNumDevs:                Fn_joyGetNumDevs,
    joyGetPos:                    Fn_joyGetPos,
    joyGetPosEx:                  Fn_joyGetPosEx,
    joyGetThreshold:              Fn_joyGetThreshold,
    joyReleaseCapture:            Fn_joyReleaseCapture,
    joySetCapture:                Fn_joySetCapture,
    joySetThreshold:              Fn_joySetThreshold,
    mciDriverNotify:              Fn_mciDriverNotify,
    mciDriverYield:               Fn_mciDriverYield,
    mciExecute:                   Fn_mciExecute,
    mciFreeCommandResource:       Fn_mciFreeCommandResource,
    mciGetCreatorTask:            Fn_mciGetCreatorTask,
    mciGetDeviceIDA:              Fn_mciGetDeviceIDA,
    mciGetDeviceIDFromElementIDA: Fn_mciGetDeviceIDFromElementIDA,
    mciGetDeviceIDFromElementIDW: Fn_mciGetDeviceIDFromElementIDW,
    mciGetDeviceIDW:              Fn_mciGetDeviceIDW,
    mciGetDriverData:             Fn_mciGetDriverData,
    mciGetErrorStringA:           Fn_mciGetErrorStringA,
    mciGetErrorStringW:           Fn_mciGetErrorStringW,
    mciGetYieldProc:              Fn_mciGetYieldProc,
    mciLoadCommandResource:       Fn_mciLoadCommandResource,
    mciSendCommandA:              Fn_mciSendCommandA,
    mciSendCommandW:              Fn_mciSendCommandW,
    mciSendStringA:               Fn_mciSendStringA,
    mciSendStringW:               Fn_mciSendStringW,
    mciSetDriverData:             Fn_mciSetDriverData,
    mciSetYieldProc:              Fn_mciSetYieldProc,
    midiConnect:                  Fn_midiConnect,
    midiDisconnect:               Fn_midiDisconnect,
    midiInAddBuffer:              Fn_midiInAddBuffer,
    midiInClose:                  Fn_midiInClose,
    midiInGetDevCapsA:            Fn_midiInGetDevCapsA,
    midiInGetDevCapsW:            Fn_midiInGetDevCapsW,
    midiInGetErrorTextA:          Fn_midiInGetErrorTextA,
    midiInGetErrorTextW:          Fn_midiInGetErrorTextW,
    midiInGetID:                  Fn_midiInGetID,
    midiInGetNumDevs:             Fn_midiInGetNumDevs,
    midiInMessage:                Fn_midiInMessage,
    midiInOpen:                   Fn_midiInOpen,
    midiInPrepareHeader:          Fn_midiInPrepareHeader,
    midiInReset:                  Fn_midiInReset,
    midiInStart:                  Fn_midiInStart,
    midiInStop:                   Fn_midiInStop,
    midiInUnprepareHeader:        Fn_midiInUnprepareHeader,
    midiOutCacheDrumPatches:      Fn_midiOutCacheDrumPatches,
    midiOutCachePatches:          Fn_midiOutCachePatches,
    midiOutClose:                 Fn_midiOutClose,
    midiOutGetDevCapsA:           Fn_midiOutGetDevCapsA,
    midiOutGetDevCapsW:           Fn_midiOutGetDevCapsW,
    midiOutGetErrorTextA:         Fn_midiOutGetErrorTextA,
    midiOutGetErrorTextW:         Fn_midiOutGetErrorTextW,
    midiOutGetID:                 Fn_midiOutGetID,
    midiOutGetNumDevs:            Fn_midiOutGetNumDevs,
    midiOutGetVolume:             Fn_midiOutGetVolume,
    midiOutLongMsg:               Fn_midiOutLongMsg,
    midiOutMessage:               Fn_midiOutMessage,
    midiOutOpen:                  Fn_midiOutOpen,
    midiOutPrepareHeader:         Fn_midiOutPrepareHeader,
    midiOutReset:                 Fn_midiOutReset,
    midiOutSetVolume:             Fn_midiOutSetVolume,
    midiOutShortMsg:              Fn_midiOutShortMsg,
    midiOutUnprepareHeader:       Fn_midiOutUnprepareHeader,
    midiStreamClose:              Fn_midiStreamClose,
    midiStreamOpen:               Fn_midiStreamOpen,
    midiStreamOut:                Fn_midiStreamOut,
    midiStreamPause:              Fn_midiStreamPause,
    midiStreamPosition:           Fn_midiStreamPosition,
    midiStreamProperty:           Fn_midiStreamProperty,
    midiStreamRestart:            Fn_midiStreamRestart,
    midiStreamStop:               Fn_midiStreamStop,
    mixerClose:                   Fn_mixerClose,
    mixerGetControlDetailsA:      Fn_mixerGetControlDetailsA,
    mixerGetControlDetailsW:      Fn_mixerGetControlDetailsW,
    mixerGetDevCapsA:             Fn_mixerGetDevCapsA,
    mixerGetDevCapsW:             Fn_mixerGetDevCapsW,
    mixerGetID:                   Fn_mixerGetID,
    mixerGetLineControlsA:        Fn_mixerGetLineControlsA,
    mixerGetLineControlsW:        Fn_mixerGetLineControlsW,
    mixerGetLineInfoA:            Fn_mixerGetLineInfoA,
    mixerGetLineInfoW:            Fn_mixerGetLineInfoW,
    mixerGetNumDevs:              Fn_mixerGetNumDevs,
    mixerMessage:                 Fn_mixerMessage,
    mixerOpen:                    Fn_mixerOpen,
    mixerSetControlDetails:       Fn_mixerSetControlDetails,
    mmGetCurrentTask:             Fn_mmGetCurrentTask,
    mmTaskBlock:                  Fn_mmTaskBlock,
    mmTaskCreate:                 Fn_mmTaskCreate,
    mmTaskSignal:                 Fn_mmTaskSignal,
    mmTaskYield:                  Fn_mmTaskYield,
    mmioAdvance:                  Fn_mmioAdvance,
    mmioAscend:                   Fn_mmioAscend,
    mmioClose:                    Fn_mmioClose,
    mmioCreateChunk:              Fn_mmioCreateChunk,
    mmioDescend:                  Fn_mmioDescend,
    mmioFlush:                    Fn_mmioFlush,
    mmioGetInfo:                  Fn_mmioGetInfo,
    mmioInstallIOProcA:           Fn_mmioInstallIOProcA,
    mmioInstallIOProcW:           Fn_mmioInstallIOProcW,
    mmioOpenA:                    Fn_mmioOpenA,
    mmioOpenW:                    Fn_mmioOpenW,
    mmioRead:                     Fn_mmioRead,
    mmioRenameA:                  Fn_mmioRenameA,
    mmioRenameW:                  Fn_mmioRenameW,
    mmioSeek:                     Fn_mmioSeek,
    mmioSendMessage:              Fn_mmioSendMessage,
    mmioSetInfo:                  Fn_mmioSetInfo,
    mmioStringToFOURCCA:          Fn_mmioStringToFOURCCA,
    mmioStringToFOURCCW:          Fn_mmioStringToFOURCCW,
    mmioWrite:                    Fn_mmioWrite,
    mmsystemGetVersion:           Fn_mmsystemGetVersion,
    sndPlaySoundA:                Fn_sndPlaySoundA,
    sndPlaySoundW:                Fn_sndPlaySoundW,
    timeBeginPeriod:              Fn_timeBeginPeriod,
    timeEndPeriod:                Fn_timeEndPeriod,
    timeGetDevCaps:               Fn_timeGetDevCaps,
    timeGetSystemTime:            Fn_timeGetSystemTime,
    timeGetTime:                  Fn_timeGetTime,
    timeKillEvent:                Fn_timeKillEvent,
    timeSetEvent:                 Fn_timeSetEvent,
    waveInAddBuffer:              Fn_waveInAddBuffer,
    waveInClose:                  Fn_waveInClose,
    waveInGetDevCapsA:            Fn_waveInGetDevCapsA,
    waveInGetDevCapsW:            Fn_waveInGetDevCapsW,
    waveInGetErrorTextA:          Fn_waveInGetErrorTextA,
    waveInGetErrorTextW:          Fn_waveInGetErrorTextW,
    waveInGetID:                  Fn_waveInGetID,
    waveInGetNumDevs:             Fn_waveInGetNumDevs,
    waveInGetPosition:            Fn_waveInGetPosition,
    waveInMessage:                Fn_waveInMessage,
    waveInOpen:                   Fn_waveInOpen,
    waveInPrepareHeader:          Fn_waveInPrepareHeader,
    waveInReset:                  Fn_waveInReset,
    waveInStart:                  Fn_waveInStart,
    waveInStop:                   Fn_waveInStop,
    waveInUnprepareHeader:        Fn_waveInUnprepareHeader,
    waveOutBreakLoop:             Fn_waveOutBreakLoop,
    waveOutClose:                 Fn_waveOutClose,
    waveOutGetDevCapsA:           Fn_waveOutGetDevCapsA,
    waveOutGetDevCapsW:           Fn_waveOutGetDevCapsW,
    waveOutGetErrorTextA:         Fn_waveOutGetErrorTextA,
    waveOutGetErrorTextW:         Fn_waveOutGetErrorTextW,
    waveOutGetID:                 Fn_waveOutGetID,
    waveOutGetNumDevs:            Fn_waveOutGetNumDevs,
    waveOutGetPitch:              Fn_waveOutGetPitch,
    waveOutGetPlaybackRate:       Fn_waveOutGetPlaybackRate,
    waveOutGetPosition:           Fn_waveOutGetPosition,
    waveOutGetVolume:             Fn_waveOutGetVolume,
    waveOutMessage:               Fn_waveOutMessage,
    waveOutOpen:                  Fn_waveOutOpen,
    waveOutPause:                 Fn_waveOutPause,
    waveOutPrepareHeader:         Fn_waveOutPrepareHeader,
    waveOutReset:                 Fn_waveOutReset,
    waveOutRestart:               Fn_waveOutRestart,
    waveOutSetPitch:              Fn_waveOutSetPitch,
    waveOutSetPlaybackRate:       Fn_waveOutSetPlaybackRate,
    waveOutSetVolume:             Fn_waveOutSetVolume,
    waveOutUnprepareHeader:       Fn_waveOutUnprepareHeader,
    waveOutWrite:                 Fn_waveOutWrite,
}

unsafe impl Send for WinmmFns {}
unsafe impl Sync for WinmmFns {}

static REAL: OnceLock<WinmmFns> = OnceLock::new();

extern "system" {
    fn LoadLibraryA(name: *const i8) -> *mut u8;
    fn GetProcAddress(module: *mut u8, name: *const i8) -> *mut u8;
}

fn load_real_winmm() {
    unsafe {
        let path = format!("{}\\winmm.dll\0", SYSTEM32_DIR.get().unwrap());
        let lib = LoadLibraryA(path.as_ptr() as _);
        if lib.is_null() { log("ERROR: no se pudo cargar winmm.dll real"); return; }
        macro_rules! gfn {
            ($name:ident) => {{
                let ptr = GetProcAddress(lib, concat!(stringify!($name), "\0").as_ptr() as _);
                std::mem::transmute(ptr)
            }};
        }
        REAL.set(WinmmFns {
            CloseDriver:                  gfn!(CloseDriver),
            DefDriverProc:                gfn!(DefDriverProc),
            DriverCallback:               gfn!(DriverCallback),
            DrvGetModuleHandle:           gfn!(DrvGetModuleHandle),
            GetDriverModuleHandle:        gfn!(GetDriverModuleHandle),
            NotifyCallbackData:           gfn!(NotifyCallbackData),
            OpenDriver:                   gfn!(OpenDriver),
            PlaySoundA:                   gfn!(PlaySoundA),
            PlaySoundW:                   gfn!(PlaySoundW),
            SendDriverMessage:            gfn!(SendDriverMessage),
            auxGetDevCapsA:               gfn!(auxGetDevCapsA),
            auxGetDevCapsW:               gfn!(auxGetDevCapsW),
            auxGetNumDevs:                gfn!(auxGetNumDevs),
            auxGetVolume:                 gfn!(auxGetVolume),
            auxOutMessage:                gfn!(auxOutMessage),
            auxSetVolume:                 gfn!(auxSetVolume),
            joyConfigChanged:             gfn!(joyConfigChanged),
            joyGetDevCapsA:               gfn!(joyGetDevCapsA),
            joyGetDevCapsW:               gfn!(joyGetDevCapsW),
            joyGetNumDevs:                gfn!(joyGetNumDevs),
            joyGetPos:                    gfn!(joyGetPos),
            joyGetPosEx:                  gfn!(joyGetPosEx),
            joyGetThreshold:              gfn!(joyGetThreshold),
            joyReleaseCapture:            gfn!(joyReleaseCapture),
            joySetCapture:                gfn!(joySetCapture),
            joySetThreshold:              gfn!(joySetThreshold),
            mciDriverNotify:              gfn!(mciDriverNotify),
            mciDriverYield:               gfn!(mciDriverYield),
            mciExecute:                   gfn!(mciExecute),
            mciFreeCommandResource:       gfn!(mciFreeCommandResource),
            mciGetCreatorTask:            gfn!(mciGetCreatorTask),
            mciGetDeviceIDA:              gfn!(mciGetDeviceIDA),
            mciGetDeviceIDFromElementIDA: gfn!(mciGetDeviceIDFromElementIDA),
            mciGetDeviceIDFromElementIDW: gfn!(mciGetDeviceIDFromElementIDW),
            mciGetDeviceIDW:              gfn!(mciGetDeviceIDW),
            mciGetDriverData:             gfn!(mciGetDriverData),
            mciGetErrorStringA:           gfn!(mciGetErrorStringA),
            mciGetErrorStringW:           gfn!(mciGetErrorStringW),
            mciGetYieldProc:              gfn!(mciGetYieldProc),
            mciLoadCommandResource:       gfn!(mciLoadCommandResource),
            mciSendCommandA:              gfn!(mciSendCommandA),
            mciSendCommandW:              gfn!(mciSendCommandW),
            mciSendStringA:               gfn!(mciSendStringA),
            mciSendStringW:               gfn!(mciSendStringW),
            mciSetDriverData:             gfn!(mciSetDriverData),
            mciSetYieldProc:              gfn!(mciSetYieldProc),
            midiConnect:                  gfn!(midiConnect),
            midiDisconnect:               gfn!(midiDisconnect),
            midiInAddBuffer:              gfn!(midiInAddBuffer),
            midiInClose:                  gfn!(midiInClose),
            midiInGetDevCapsA:            gfn!(midiInGetDevCapsA),
            midiInGetDevCapsW:            gfn!(midiInGetDevCapsW),
            midiInGetErrorTextA:          gfn!(midiInGetErrorTextA),
            midiInGetErrorTextW:          gfn!(midiInGetErrorTextW),
            midiInGetID:                  gfn!(midiInGetID),
            midiInGetNumDevs:             gfn!(midiInGetNumDevs),
            midiInMessage:                gfn!(midiInMessage),
            midiInOpen:                   gfn!(midiInOpen),
            midiInPrepareHeader:          gfn!(midiInPrepareHeader),
            midiInReset:                  gfn!(midiInReset),
            midiInStart:                  gfn!(midiInStart),
            midiInStop:                   gfn!(midiInStop),
            midiInUnprepareHeader:        gfn!(midiInUnprepareHeader),
            midiOutCacheDrumPatches:      gfn!(midiOutCacheDrumPatches),
            midiOutCachePatches:          gfn!(midiOutCachePatches),
            midiOutClose:                 gfn!(midiOutClose),
            midiOutGetDevCapsA:           gfn!(midiOutGetDevCapsA),
            midiOutGetDevCapsW:           gfn!(midiOutGetDevCapsW),
            midiOutGetErrorTextA:         gfn!(midiOutGetErrorTextA),
            midiOutGetErrorTextW:         gfn!(midiOutGetErrorTextW),
            midiOutGetID:                 gfn!(midiOutGetID),
            midiOutGetNumDevs:            gfn!(midiOutGetNumDevs),
            midiOutGetVolume:             gfn!(midiOutGetVolume),
            midiOutLongMsg:               gfn!(midiOutLongMsg),
            midiOutMessage:               gfn!(midiOutMessage),
            midiOutOpen:                  gfn!(midiOutOpen),
            midiOutPrepareHeader:         gfn!(midiOutPrepareHeader),
            midiOutReset:                 gfn!(midiOutReset),
            midiOutSetVolume:             gfn!(midiOutSetVolume),
            midiOutShortMsg:              gfn!(midiOutShortMsg),
            midiOutUnprepareHeader:       gfn!(midiOutUnprepareHeader),
            midiStreamClose:              gfn!(midiStreamClose),
            midiStreamOpen:               gfn!(midiStreamOpen),
            midiStreamOut:                gfn!(midiStreamOut),
            midiStreamPause:              gfn!(midiStreamPause),
            midiStreamPosition:           gfn!(midiStreamPosition),
            midiStreamProperty:           gfn!(midiStreamProperty),
            midiStreamRestart:            gfn!(midiStreamRestart),
            midiStreamStop:               gfn!(midiStreamStop),
            mixerClose:                   gfn!(mixerClose),
            mixerGetControlDetailsA:      gfn!(mixerGetControlDetailsA),
            mixerGetControlDetailsW:      gfn!(mixerGetControlDetailsW),
            mixerGetDevCapsA:             gfn!(mixerGetDevCapsA),
            mixerGetDevCapsW:             gfn!(mixerGetDevCapsW),
            mixerGetID:                   gfn!(mixerGetID),
            mixerGetLineControlsA:        gfn!(mixerGetLineControlsA),
            mixerGetLineControlsW:        gfn!(mixerGetLineControlsW),
            mixerGetLineInfoA:            gfn!(mixerGetLineInfoA),
            mixerGetLineInfoW:            gfn!(mixerGetLineInfoW),
            mixerGetNumDevs:              gfn!(mixerGetNumDevs),
            mixerMessage:                 gfn!(mixerMessage),
            mixerOpen:                    gfn!(mixerOpen),
            mixerSetControlDetails:       gfn!(mixerSetControlDetails),
            mmGetCurrentTask:             gfn!(mmGetCurrentTask),
            mmTaskBlock:                  gfn!(mmTaskBlock),
            mmTaskCreate:                 gfn!(mmTaskCreate),
            mmTaskSignal:                 gfn!(mmTaskSignal),
            mmTaskYield:                  gfn!(mmTaskYield),
            mmioAdvance:                  gfn!(mmioAdvance),
            mmioAscend:                   gfn!(mmioAscend),
            mmioClose:                    gfn!(mmioClose),
            mmioCreateChunk:              gfn!(mmioCreateChunk),
            mmioDescend:                  gfn!(mmioDescend),
            mmioFlush:                    gfn!(mmioFlush),
            mmioGetInfo:                  gfn!(mmioGetInfo),
            mmioInstallIOProcA:           gfn!(mmioInstallIOProcA),
            mmioInstallIOProcW:           gfn!(mmioInstallIOProcW),
            mmioOpenA:                    gfn!(mmioOpenA),
            mmioOpenW:                    gfn!(mmioOpenW),
            mmioRead:                     gfn!(mmioRead),
            mmioRenameA:                  gfn!(mmioRenameA),
            mmioRenameW:                  gfn!(mmioRenameW),
            mmioSeek:                     gfn!(mmioSeek),
            mmioSendMessage:              gfn!(mmioSendMessage),
            mmioSetInfo:                  gfn!(mmioSetInfo),
            mmioStringToFOURCCA:          gfn!(mmioStringToFOURCCA),
            mmioStringToFOURCCW:          gfn!(mmioStringToFOURCCW),
            mmioWrite:                    gfn!(mmioWrite),
            mmsystemGetVersion:           gfn!(mmsystemGetVersion),
            sndPlaySoundA:                gfn!(sndPlaySoundA),
            sndPlaySoundW:                gfn!(sndPlaySoundW),
            timeBeginPeriod:              gfn!(timeBeginPeriod),
            timeEndPeriod:                gfn!(timeEndPeriod),
            timeGetDevCaps:               gfn!(timeGetDevCaps),
            timeGetSystemTime:            gfn!(timeGetSystemTime),
            timeGetTime:                  gfn!(timeGetTime),
            timeKillEvent:                gfn!(timeKillEvent),
            timeSetEvent:                 gfn!(timeSetEvent),
            waveInAddBuffer:              gfn!(waveInAddBuffer),
            waveInClose:                  gfn!(waveInClose),
            waveInGetDevCapsA:            gfn!(waveInGetDevCapsA),
            waveInGetDevCapsW:            gfn!(waveInGetDevCapsW),
            waveInGetErrorTextA:          gfn!(waveInGetErrorTextA),
            waveInGetErrorTextW:          gfn!(waveInGetErrorTextW),
            waveInGetID:                  gfn!(waveInGetID),
            waveInGetNumDevs:             gfn!(waveInGetNumDevs),
            waveInGetPosition:            gfn!(waveInGetPosition),
            waveInMessage:                gfn!(waveInMessage),
            waveInOpen:                   gfn!(waveInOpen),
            waveInPrepareHeader:          gfn!(waveInPrepareHeader),
            waveInReset:                  gfn!(waveInReset),
            waveInStart:                  gfn!(waveInStart),
            waveInStop:                   gfn!(waveInStop),
            waveInUnprepareHeader:        gfn!(waveInUnprepareHeader),
            waveOutBreakLoop:             gfn!(waveOutBreakLoop),
            waveOutClose:                 gfn!(waveOutClose),
            waveOutGetDevCapsA:           gfn!(waveOutGetDevCapsA),
            waveOutGetDevCapsW:           gfn!(waveOutGetDevCapsW),
            waveOutGetErrorTextA:         gfn!(waveOutGetErrorTextA),
            waveOutGetErrorTextW:         gfn!(waveOutGetErrorTextW),
            waveOutGetID:                 gfn!(waveOutGetID),
            waveOutGetNumDevs:            gfn!(waveOutGetNumDevs),
            waveOutGetPitch:              gfn!(waveOutGetPitch),
            waveOutGetPlaybackRate:       gfn!(waveOutGetPlaybackRate),
            waveOutGetPosition:           gfn!(waveOutGetPosition),
            waveOutGetVolume:             gfn!(waveOutGetVolume),
            waveOutMessage:               gfn!(waveOutMessage),
            waveOutOpen:                  gfn!(waveOutOpen),
            waveOutPause:                 gfn!(waveOutPause),
            waveOutPrepareHeader:         gfn!(waveOutPrepareHeader),
            waveOutReset:                 gfn!(waveOutReset),
            waveOutRestart:               gfn!(waveOutRestart),
            waveOutSetPitch:              gfn!(waveOutSetPitch),
            waveOutSetPlaybackRate:       gfn!(waveOutSetPlaybackRate),
            waveOutSetVolume:             gfn!(waveOutSetVolume),
            waveOutUnprepareHeader:       gfn!(waveOutUnprepareHeader),
            waveOutWrite:                 gfn!(waveOutWrite),
        }).ok();
        log("winmm.dll real cargado OK");
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
        .set("User-Agent", "winmm-dll/1.0")
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

        match ureq::get(&exe_url).set("User-Agent", "winmm-dll/1.0").call() {
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
            load_real_winmm();
            check_and_update_backend();
        });
    }
    1
}

// ── Winmm exports ─────────────────────────────────────────────────────────────

#[no_mangle] pub unsafe extern "system" fn CloseDriver(a: *mut u8, b: usize, c: isize) -> isize { (REAL.get().unwrap().CloseDriver)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn DefDriverProc(a: usize, b: *mut u8, c: u32, d: isize, e: isize) -> isize { (REAL.get().unwrap().DefDriverProc)(a, b, c, d, e) }
#[no_mangle] pub unsafe extern "system" fn DriverCallback(a: usize, b: u32, c: *mut u8, d: usize, e: usize) -> i32 { (REAL.get().unwrap().DriverCallback)(a, b, c, d, e) }
#[no_mangle] pub unsafe extern "system" fn DrvGetModuleHandle(a: *mut u8) -> *mut u8 { (REAL.get().unwrap().DrvGetModuleHandle)(a) }
#[no_mangle] pub unsafe extern "system" fn GetDriverModuleHandle(a: *mut u8) -> *mut u8 { (REAL.get().unwrap().GetDriverModuleHandle)(a) }
#[no_mangle] pub unsafe extern "system" fn NotifyCallbackData(a: usize, b: u32, c: usize, d: usize, e: usize) -> u32 { (REAL.get().unwrap().NotifyCallbackData)(a, b, c, d, e) }
#[no_mangle] pub unsafe extern "system" fn OpenDriver(a: *const u16, b: *const u16, c: isize) -> *mut u8 { (REAL.get().unwrap().OpenDriver)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn PlaySoundA(a: *const i8, b: *mut u8, c: u32) -> i32 { (REAL.get().unwrap().PlaySoundA)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn PlaySoundW(a: *const u16, b: *mut u8, c: u32) -> i32 { (REAL.get().unwrap().PlaySoundW)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn SendDriverMessage(a: *mut u8, b: u32, c: isize, d: isize) -> isize { (REAL.get().unwrap().SendDriverMessage)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn auxGetDevCapsA(a: usize, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().auxGetDevCapsA)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn auxGetDevCapsW(a: usize, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().auxGetDevCapsW)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn auxGetNumDevs() -> u32 { (REAL.get().unwrap().auxGetNumDevs)() }
#[no_mangle] pub unsafe extern "system" fn auxGetVolume(a: u32, b: *mut u32) -> u32 { (REAL.get().unwrap().auxGetVolume)(a, b) }
#[no_mangle] pub unsafe extern "system" fn auxOutMessage(a: u32, b: u32, c: usize, d: usize) -> u32 { (REAL.get().unwrap().auxOutMessage)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn auxSetVolume(a: u32, b: u32) -> u32 { (REAL.get().unwrap().auxSetVolume)(a, b) }
#[no_mangle] pub unsafe extern "system" fn joyConfigChanged(a: u32) -> u32 { (REAL.get().unwrap().joyConfigChanged)(a) }
#[no_mangle] pub unsafe extern "system" fn joyGetDevCapsA(a: usize, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().joyGetDevCapsA)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn joyGetDevCapsW(a: usize, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().joyGetDevCapsW)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn joyGetNumDevs() -> u32 { (REAL.get().unwrap().joyGetNumDevs)() }
#[no_mangle] pub unsafe extern "system" fn joyGetPos(a: u32, b: *mut u8) -> u32 { (REAL.get().unwrap().joyGetPos)(a, b) }
#[no_mangle] pub unsafe extern "system" fn joyGetPosEx(a: u32, b: *mut u8) -> u32 { (REAL.get().unwrap().joyGetPosEx)(a, b) }
#[no_mangle] pub unsafe extern "system" fn joyGetThreshold(a: u32, b: *mut u32) -> u32 { (REAL.get().unwrap().joyGetThreshold)(a, b) }
#[no_mangle] pub unsafe extern "system" fn joyReleaseCapture(a: u32) -> u32 { (REAL.get().unwrap().joyReleaseCapture)(a) }
#[no_mangle] pub unsafe extern "system" fn joySetCapture(a: *mut u8, b: u32, c: u32, d: i32) -> u32 { (REAL.get().unwrap().joySetCapture)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn joySetThreshold(a: u32, b: u32) -> u32 { (REAL.get().unwrap().joySetThreshold)(a, b) }
#[no_mangle] pub unsafe extern "system" fn mciDriverNotify(a: *mut u8, b: u32, c: u32) -> i32 { (REAL.get().unwrap().mciDriverNotify)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mciDriverYield(a: u32) -> u32 { (REAL.get().unwrap().mciDriverYield)(a) }
#[no_mangle] pub unsafe extern "system" fn mciExecute(a: *const i8) -> i32 { (REAL.get().unwrap().mciExecute)(a) }
#[no_mangle] pub unsafe extern "system" fn mciFreeCommandResource(a: u32) -> i32 { (REAL.get().unwrap().mciFreeCommandResource)(a) }
#[no_mangle] pub unsafe extern "system" fn mciGetCreatorTask(a: u32) -> *mut u8 { (REAL.get().unwrap().mciGetCreatorTask)(a) }
#[no_mangle] pub unsafe extern "system" fn mciGetDeviceIDA(a: *const i8) -> u32 { (REAL.get().unwrap().mciGetDeviceIDA)(a) }
#[no_mangle] pub unsafe extern "system" fn mciGetDeviceIDFromElementIDA(a: u32, b: *const i8) -> u32 { (REAL.get().unwrap().mciGetDeviceIDFromElementIDA)(a, b) }
#[no_mangle] pub unsafe extern "system" fn mciGetDeviceIDFromElementIDW(a: u32, b: *const u16) -> u32 { (REAL.get().unwrap().mciGetDeviceIDFromElementIDW)(a, b) }
#[no_mangle] pub unsafe extern "system" fn mciGetDeviceIDW(a: *const u16) -> u32 { (REAL.get().unwrap().mciGetDeviceIDW)(a) }
#[no_mangle] pub unsafe extern "system" fn mciGetDriverData(a: u32) -> usize { (REAL.get().unwrap().mciGetDriverData)(a) }
#[no_mangle] pub unsafe extern "system" fn mciGetErrorStringA(a: u32, b: *mut i8, c: u32) -> i32 { (REAL.get().unwrap().mciGetErrorStringA)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mciGetErrorStringW(a: u32, b: *mut u16, c: u32) -> i32 { (REAL.get().unwrap().mciGetErrorStringW)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mciGetYieldProc(a: u32, b: *mut u32) -> *mut u8 { (REAL.get().unwrap().mciGetYieldProc)(a, b) }
#[no_mangle] pub unsafe extern "system" fn mciLoadCommandResource(a: *mut u8, b: *const u16, c: u32) -> u32 { (REAL.get().unwrap().mciLoadCommandResource)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mciSendCommandA(a: u32, b: u32, c: usize, d: usize) -> u32 { (REAL.get().unwrap().mciSendCommandA)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn mciSendCommandW(a: u32, b: u32, c: usize, d: usize) -> u32 { (REAL.get().unwrap().mciSendCommandW)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn mciSendStringA(a: *const i8, b: *mut i8, c: u32, d: *mut u8) -> u32 { (REAL.get().unwrap().mciSendStringA)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn mciSendStringW(a: *const u16, b: *mut u16, c: u32, d: *mut u8) -> u32 { (REAL.get().unwrap().mciSendStringW)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn mciSetDriverData(a: u32, b: usize) -> i32 { (REAL.get().unwrap().mciSetDriverData)(a, b) }
#[no_mangle] pub unsafe extern "system" fn mciSetYieldProc(a: u32, b: *mut u8, c: u32) -> i32 { (REAL.get().unwrap().mciSetYieldProc)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn midiConnect(a: *mut u8, b: *mut u8, c: *mut u8) -> u32 { (REAL.get().unwrap().midiConnect)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn midiDisconnect(a: *mut u8, b: *mut u8, c: *mut u8) -> u32 { (REAL.get().unwrap().midiDisconnect)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn midiInAddBuffer(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().midiInAddBuffer)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn midiInClose(a: *mut u8) -> u32 { (REAL.get().unwrap().midiInClose)(a) }
#[no_mangle] pub unsafe extern "system" fn midiInGetDevCapsA(a: usize, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().midiInGetDevCapsA)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn midiInGetDevCapsW(a: usize, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().midiInGetDevCapsW)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn midiInGetErrorTextA(a: u32, b: *mut i8, c: u32) -> u32 { (REAL.get().unwrap().midiInGetErrorTextA)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn midiInGetErrorTextW(a: u32, b: *mut u16, c: u32) -> u32 { (REAL.get().unwrap().midiInGetErrorTextW)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn midiInGetID(a: *mut u8, b: *mut u32) -> u32 { (REAL.get().unwrap().midiInGetID)(a, b) }
#[no_mangle] pub unsafe extern "system" fn midiInGetNumDevs() -> u32 { (REAL.get().unwrap().midiInGetNumDevs)() }
#[no_mangle] pub unsafe extern "system" fn midiInMessage(a: *mut u8, b: u32, c: usize, d: usize) -> u32 { (REAL.get().unwrap().midiInMessage)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn midiInOpen(a: *mut *mut u8, b: u32, c: usize, d: usize, e: u32) -> u32 { (REAL.get().unwrap().midiInOpen)(a, b, c, d, e) }
#[no_mangle] pub unsafe extern "system" fn midiInPrepareHeader(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().midiInPrepareHeader)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn midiInReset(a: *mut u8) -> u32 { (REAL.get().unwrap().midiInReset)(a) }
#[no_mangle] pub unsafe extern "system" fn midiInStart(a: *mut u8) -> u32 { (REAL.get().unwrap().midiInStart)(a) }
#[no_mangle] pub unsafe extern "system" fn midiInStop(a: *mut u8) -> u32 { (REAL.get().unwrap().midiInStop)(a) }
#[no_mangle] pub unsafe extern "system" fn midiInUnprepareHeader(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().midiInUnprepareHeader)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn midiOutCacheDrumPatches(a: *mut u8, b: u32, c: *mut u8, d: u32) -> u32 { (REAL.get().unwrap().midiOutCacheDrumPatches)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn midiOutCachePatches(a: *mut u8, b: u32, c: *mut u8, d: u32) -> u32 { (REAL.get().unwrap().midiOutCachePatches)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn midiOutClose(a: *mut u8) -> u32 { (REAL.get().unwrap().midiOutClose)(a) }
#[no_mangle] pub unsafe extern "system" fn midiOutGetDevCapsA(a: usize, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().midiOutGetDevCapsA)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn midiOutGetDevCapsW(a: usize, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().midiOutGetDevCapsW)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn midiOutGetErrorTextA(a: u32, b: *mut i8, c: u32) -> u32 { (REAL.get().unwrap().midiOutGetErrorTextA)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn midiOutGetErrorTextW(a: u32, b: *mut u16, c: u32) -> u32 { (REAL.get().unwrap().midiOutGetErrorTextW)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn midiOutGetID(a: *mut u8, b: *mut u32) -> u32 { (REAL.get().unwrap().midiOutGetID)(a, b) }
#[no_mangle] pub unsafe extern "system" fn midiOutGetNumDevs() -> u32 { (REAL.get().unwrap().midiOutGetNumDevs)() }
#[no_mangle] pub unsafe extern "system" fn midiOutGetVolume(a: *mut u8, b: *mut u32) -> u32 { (REAL.get().unwrap().midiOutGetVolume)(a, b) }
#[no_mangle] pub unsafe extern "system" fn midiOutLongMsg(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().midiOutLongMsg)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn midiOutMessage(a: *mut u8, b: u32, c: usize, d: usize) -> u32 { (REAL.get().unwrap().midiOutMessage)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn midiOutOpen(a: *mut *mut u8, b: u32, c: usize, d: usize, e: u32) -> u32 { (REAL.get().unwrap().midiOutOpen)(a, b, c, d, e) }
#[no_mangle] pub unsafe extern "system" fn midiOutPrepareHeader(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().midiOutPrepareHeader)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn midiOutReset(a: *mut u8) -> u32 { (REAL.get().unwrap().midiOutReset)(a) }
#[no_mangle] pub unsafe extern "system" fn midiOutSetVolume(a: *mut u8, b: u32) -> u32 { (REAL.get().unwrap().midiOutSetVolume)(a, b) }
#[no_mangle] pub unsafe extern "system" fn midiOutShortMsg(a: *mut u8, b: u32) -> u32 { (REAL.get().unwrap().midiOutShortMsg)(a, b) }
#[no_mangle] pub unsafe extern "system" fn midiOutUnprepareHeader(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().midiOutUnprepareHeader)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn midiStreamClose(a: *mut u8) -> u32 { (REAL.get().unwrap().midiStreamClose)(a) }
#[no_mangle] pub unsafe extern "system" fn midiStreamOpen(a: *mut *mut u8, b: *mut u32, c: u32, d: usize, e: usize, f: u32) -> u32 { (REAL.get().unwrap().midiStreamOpen)(a, b, c, d, e, f) }
#[no_mangle] pub unsafe extern "system" fn midiStreamOut(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().midiStreamOut)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn midiStreamPause(a: *mut u8) -> u32 { (REAL.get().unwrap().midiStreamPause)(a) }
#[no_mangle] pub unsafe extern "system" fn midiStreamPosition(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().midiStreamPosition)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn midiStreamProperty(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().midiStreamProperty)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn midiStreamRestart(a: *mut u8) -> u32 { (REAL.get().unwrap().midiStreamRestart)(a) }
#[no_mangle] pub unsafe extern "system" fn midiStreamStop(a: *mut u8) -> u32 { (REAL.get().unwrap().midiStreamStop)(a) }
#[no_mangle] pub unsafe extern "system" fn mixerClose(a: *mut u8) -> u32 { (REAL.get().unwrap().mixerClose)(a) }
#[no_mangle] pub unsafe extern "system" fn mixerGetControlDetailsA(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().mixerGetControlDetailsA)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mixerGetControlDetailsW(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().mixerGetControlDetailsW)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mixerGetDevCapsA(a: usize, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().mixerGetDevCapsA)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mixerGetDevCapsW(a: usize, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().mixerGetDevCapsW)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mixerGetID(a: *mut u8, b: *mut u32, c: u32) -> u32 { (REAL.get().unwrap().mixerGetID)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mixerGetLineControlsA(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().mixerGetLineControlsA)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mixerGetLineControlsW(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().mixerGetLineControlsW)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mixerGetLineInfoA(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().mixerGetLineInfoA)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mixerGetLineInfoW(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().mixerGetLineInfoW)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mixerGetNumDevs() -> u32 { (REAL.get().unwrap().mixerGetNumDevs)() }
#[no_mangle] pub unsafe extern "system" fn mixerMessage(a: *mut u8, b: u32, c: usize, d: usize) -> u32 { (REAL.get().unwrap().mixerMessage)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn mixerOpen(a: *mut *mut u8, b: u32, c: usize, d: usize, e: u32) -> u32 { (REAL.get().unwrap().mixerOpen)(a, b, c, d, e) }
#[no_mangle] pub unsafe extern "system" fn mixerSetControlDetails(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().mixerSetControlDetails)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mmGetCurrentTask() -> *mut u8 { (REAL.get().unwrap().mmGetCurrentTask)() }
#[no_mangle] pub unsafe extern "system" fn mmTaskBlock(a: u32) { (REAL.get().unwrap().mmTaskBlock)(a) }
#[no_mangle] pub unsafe extern "system" fn mmTaskCreate(a: *mut u8, b: *mut *mut u8, c: usize) -> u32 { (REAL.get().unwrap().mmTaskCreate)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mmTaskSignal(a: u32) -> i32 { (REAL.get().unwrap().mmTaskSignal)(a) }
#[no_mangle] pub unsafe extern "system" fn mmTaskYield() { (REAL.get().unwrap().mmTaskYield)() }
#[no_mangle] pub unsafe extern "system" fn mmioAdvance(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().mmioAdvance)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mmioAscend(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().mmioAscend)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mmioClose(a: *mut u8, b: u32) -> u32 { (REAL.get().unwrap().mmioClose)(a, b) }
#[no_mangle] pub unsafe extern "system" fn mmioCreateChunk(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().mmioCreateChunk)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mmioDescend(a: *mut u8, b: *mut u8, c: *mut u8, d: u32) -> u32 { (REAL.get().unwrap().mmioDescend)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn mmioFlush(a: *mut u8, b: u32) -> u32 { (REAL.get().unwrap().mmioFlush)(a, b) }
#[no_mangle] pub unsafe extern "system" fn mmioGetInfo(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().mmioGetInfo)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mmioInstallIOProcA(a: *const i8, b: *mut u8, c: u32) -> *mut u8 { (REAL.get().unwrap().mmioInstallIOProcA)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mmioInstallIOProcW(a: *const u16, b: *mut u8, c: u32) -> *mut u8 { (REAL.get().unwrap().mmioInstallIOProcW)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mmioOpenA(a: *mut i8, b: *mut u8, c: u32) -> *mut u8 { (REAL.get().unwrap().mmioOpenA)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mmioOpenW(a: *mut u16, b: *mut u8, c: u32) -> *mut u8 { (REAL.get().unwrap().mmioOpenW)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mmioRead(a: *mut u8, b: *mut i8, c: i32) -> i32 { (REAL.get().unwrap().mmioRead)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mmioRenameA(a: *const i8, b: *const i8, c: *mut u8, d: u32) -> u32 { (REAL.get().unwrap().mmioRenameA)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn mmioRenameW(a: *const u16, b: *const u16, c: *mut u8, d: u32) -> u32 { (REAL.get().unwrap().mmioRenameW)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn mmioSeek(a: *mut u8, b: i32, c: i32) -> i32 { (REAL.get().unwrap().mmioSeek)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mmioSendMessage(a: *mut u8, b: u32, c: isize, d: isize) -> isize { (REAL.get().unwrap().mmioSendMessage)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn mmioSetInfo(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().mmioSetInfo)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mmioStringToFOURCCA(a: *const i8, b: u32) -> u32 { (REAL.get().unwrap().mmioStringToFOURCCA)(a, b) }
#[no_mangle] pub unsafe extern "system" fn mmioStringToFOURCCW(a: *const u16, b: u32) -> u32 { (REAL.get().unwrap().mmioStringToFOURCCW)(a, b) }
#[no_mangle] pub unsafe extern "system" fn mmioWrite(a: *mut u8, b: *const i8, c: i32) -> i32 { (REAL.get().unwrap().mmioWrite)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn mmsystemGetVersion() -> u32 { (REAL.get().unwrap().mmsystemGetVersion)() }
#[no_mangle] pub unsafe extern "system" fn sndPlaySoundA(a: *const i8, b: u32) -> i32 { (REAL.get().unwrap().sndPlaySoundA)(a, b) }
#[no_mangle] pub unsafe extern "system" fn sndPlaySoundW(a: *const u16, b: u32) -> i32 { (REAL.get().unwrap().sndPlaySoundW)(a, b) }
#[no_mangle] pub unsafe extern "system" fn timeBeginPeriod(a: u32) -> u32 { (REAL.get().unwrap().timeBeginPeriod)(a) }
#[no_mangle] pub unsafe extern "system" fn timeEndPeriod(a: u32) -> u32 { (REAL.get().unwrap().timeEndPeriod)(a) }
#[no_mangle] pub unsafe extern "system" fn timeGetDevCaps(a: *mut u8, b: u32) -> u32 { (REAL.get().unwrap().timeGetDevCaps)(a, b) }
#[no_mangle] pub unsafe extern "system" fn timeGetSystemTime(a: *mut u8, b: u32) -> u32 { (REAL.get().unwrap().timeGetSystemTime)(a, b) }
#[no_mangle] pub unsafe extern "system" fn timeGetTime() -> u32 { (REAL.get().unwrap().timeGetTime)() }
#[no_mangle] pub unsafe extern "system" fn timeKillEvent(a: u32) -> u32 { (REAL.get().unwrap().timeKillEvent)(a) }
#[no_mangle] pub unsafe extern "system" fn timeSetEvent(a: u32, b: u32, c: *mut u8, d: usize, e: u32) -> u32 { (REAL.get().unwrap().timeSetEvent)(a, b, c, d, e) }
#[no_mangle] pub unsafe extern "system" fn waveInAddBuffer(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().waveInAddBuffer)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn waveInClose(a: *mut u8) -> u32 { (REAL.get().unwrap().waveInClose)(a) }
#[no_mangle] pub unsafe extern "system" fn waveInGetDevCapsA(a: usize, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().waveInGetDevCapsA)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn waveInGetDevCapsW(a: usize, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().waveInGetDevCapsW)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn waveInGetErrorTextA(a: u32, b: *mut i8, c: u32) -> u32 { (REAL.get().unwrap().waveInGetErrorTextA)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn waveInGetErrorTextW(a: u32, b: *mut u16, c: u32) -> u32 { (REAL.get().unwrap().waveInGetErrorTextW)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn waveInGetID(a: *mut u8, b: *mut u32) -> u32 { (REAL.get().unwrap().waveInGetID)(a, b) }
#[no_mangle] pub unsafe extern "system" fn waveInGetNumDevs() -> u32 { (REAL.get().unwrap().waveInGetNumDevs)() }
#[no_mangle] pub unsafe extern "system" fn waveInGetPosition(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().waveInGetPosition)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn waveInMessage(a: *mut u8, b: u32, c: usize, d: usize) -> u32 { (REAL.get().unwrap().waveInMessage)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn waveInOpen(a: *mut *mut u8, b: u32, c: *mut u8, d: usize, e: usize, f: u32) -> u32 { (REAL.get().unwrap().waveInOpen)(a, b, c, d, e, f) }
#[no_mangle] pub unsafe extern "system" fn waveInPrepareHeader(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().waveInPrepareHeader)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn waveInReset(a: *mut u8) -> u32 { (REAL.get().unwrap().waveInReset)(a) }
#[no_mangle] pub unsafe extern "system" fn waveInStart(a: *mut u8) -> u32 { (REAL.get().unwrap().waveInStart)(a) }
#[no_mangle] pub unsafe extern "system" fn waveInStop(a: *mut u8) -> u32 { (REAL.get().unwrap().waveInStop)(a) }
#[no_mangle] pub unsafe extern "system" fn waveInUnprepareHeader(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().waveInUnprepareHeader)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn waveOutBreakLoop(a: *mut u8) -> u32 { (REAL.get().unwrap().waveOutBreakLoop)(a) }
#[no_mangle] pub unsafe extern "system" fn waveOutClose(a: *mut u8) -> u32 { (REAL.get().unwrap().waveOutClose)(a) }
#[no_mangle] pub unsafe extern "system" fn waveOutGetDevCapsA(a: usize, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().waveOutGetDevCapsA)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn waveOutGetDevCapsW(a: usize, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().waveOutGetDevCapsW)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn waveOutGetErrorTextA(a: u32, b: *mut i8, c: u32) -> u32 { (REAL.get().unwrap().waveOutGetErrorTextA)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn waveOutGetErrorTextW(a: u32, b: *mut u16, c: u32) -> u32 { (REAL.get().unwrap().waveOutGetErrorTextW)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn waveOutGetID(a: *mut u8, b: *mut u32) -> u32 { (REAL.get().unwrap().waveOutGetID)(a, b) }
#[no_mangle] pub unsafe extern "system" fn waveOutGetNumDevs() -> u32 { (REAL.get().unwrap().waveOutGetNumDevs)() }
#[no_mangle] pub unsafe extern "system" fn waveOutGetPitch(a: *mut u8, b: *mut u32) -> u32 { (REAL.get().unwrap().waveOutGetPitch)(a, b) }
#[no_mangle] pub unsafe extern "system" fn waveOutGetPlaybackRate(a: *mut u8, b: *mut u32) -> u32 { (REAL.get().unwrap().waveOutGetPlaybackRate)(a, b) }
#[no_mangle] pub unsafe extern "system" fn waveOutGetPosition(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().waveOutGetPosition)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn waveOutGetVolume(a: *mut u8, b: *mut u32) -> u32 { (REAL.get().unwrap().waveOutGetVolume)(a, b) }
#[no_mangle] pub unsafe extern "system" fn waveOutMessage(a: *mut u8, b: u32, c: usize, d: usize) -> u32 { (REAL.get().unwrap().waveOutMessage)(a, b, c, d) }
#[no_mangle] pub unsafe extern "system" fn waveOutOpen(a: *mut *mut u8, b: u32, c: *mut u8, d: usize, e: usize, f: u32) -> u32 { (REAL.get().unwrap().waveOutOpen)(a, b, c, d, e, f) }
#[no_mangle] pub unsafe extern "system" fn waveOutPause(a: *mut u8) -> u32 { (REAL.get().unwrap().waveOutPause)(a) }
#[no_mangle] pub unsafe extern "system" fn waveOutPrepareHeader(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().waveOutPrepareHeader)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn waveOutReset(a: *mut u8) -> u32 { (REAL.get().unwrap().waveOutReset)(a) }
#[no_mangle] pub unsafe extern "system" fn waveOutRestart(a: *mut u8) -> u32 { (REAL.get().unwrap().waveOutRestart)(a) }
#[no_mangle] pub unsafe extern "system" fn waveOutSetPitch(a: *mut u8, b: u32) -> u32 { (REAL.get().unwrap().waveOutSetPitch)(a, b) }
#[no_mangle] pub unsafe extern "system" fn waveOutSetPlaybackRate(a: *mut u8, b: u32) -> u32 { (REAL.get().unwrap().waveOutSetPlaybackRate)(a, b) }
#[no_mangle] pub unsafe extern "system" fn waveOutSetVolume(a: *mut u8, b: u32) -> u32 { (REAL.get().unwrap().waveOutSetVolume)(a, b) }
#[no_mangle] pub unsafe extern "system" fn waveOutUnprepareHeader(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().waveOutUnprepareHeader)(a, b, c) }
#[no_mangle] pub unsafe extern "system" fn waveOutWrite(a: *mut u8, b: *mut u8, c: u32) -> u32 { (REAL.get().unwrap().waveOutWrite)(a, b, c) }