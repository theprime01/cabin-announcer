//! Minimal SimConnect bindings. SimConnect.dll is loaded at runtime, so the app
//! starts (and shows a clear message) even when the DLL is missing.

use std::ffi::{CString, c_char, c_void};
use std::path::{Path, PathBuf};

use libloading::Library;

type Handle = *mut c_void;
type HResult = i32;

pub const OBJECT_ID_USER: u32 = 0;
pub const DATATYPE_FLOAT64: u32 = 4;
pub const DATATYPE_STRING256: u32 = 9;
pub const PERIOD_SECOND: u32 = 4;
pub const CLIENT_DATA_PERIOD_ON_SET: u32 = 3;
pub const CLIENT_DATA_FLAG_CHANGED: u32 = 1;
const UNUSED: u32 = u32::MAX;

pub const RECV_ID_EXCEPTION: u32 = 1;
pub const RECV_ID_OPEN: u32 = 2;
pub const RECV_ID_QUIT: u32 = 3;
pub const RECV_ID_EVENT: u32 = 4;
pub const RECV_ID_SIMOBJECT_DATA: u32 = 8;
pub const RECV_ID_CLIENT_DATA: u32 = 16;

/// SIMCONNECT_RECV_SIMOBJECT_DATA (and CLIENT_DATA): a 12-byte header plus
/// seven DWORDs, then the data.
const OBJECT_DATA_OFFSET: usize = 40;

type OpenFn = unsafe extern "system" fn(*mut Handle, *const c_char, *mut c_void, u32, Handle, u32) -> HResult;
type CloseFn = unsafe extern "system" fn(Handle) -> HResult;
type AddToDataDefinitionFn =
    unsafe extern "system" fn(Handle, u32, *const c_char, *const c_char, u32, f32, u32) -> HResult;
type RequestDataOnSimObjectFn =
    unsafe extern "system" fn(Handle, u32, u32, u32, u32, u32, u32, u32, u32) -> HResult;
type GetNextDispatchFn = unsafe extern "system" fn(Handle, *mut *const u8, *mut u32) -> HResult;
type MapClientDataNameToIdFn = unsafe extern "system" fn(Handle, *const c_char, u32) -> HResult;
type AddToClientDataDefinitionFn = unsafe extern "system" fn(Handle, u32, u32, u32, f32, u32) -> HResult;
type RequestClientDataFn = unsafe extern "system" fn(Handle, u32, u32, u32, u32, u32, u32, u32, u32) -> HResult;
type SubscribeToSystemEventFn = unsafe extern "system" fn(Handle, u32, *const c_char) -> HResult;

struct Api {
    open: OpenFn,
    close: CloseFn,
    add_to_data_definition: AddToDataDefinitionFn,
    request_data_on_sim_object: RequestDataOnSimObjectFn,
    get_next_dispatch: GetNextDispatchFn,
    map_client_data_name_to_id: MapClientDataNameToIdFn,
    add_to_client_data_definition: AddToClientDataDefinitionFn,
    request_client_data: RequestClientDataFn,
    subscribe_to_system_event: SubscribeToSystemEventFn,
    // Keeps the function pointers above valid.
    _lib: Library,
}

/// Where SimConnect.dll is looked for: next to the exe, then the MSFS 2024 SDK.
pub fn dll_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        v.push(dir.join("SimConnect.dll"));
    }
    for var in ["MSFS2024_SDK", "MSFS_SDK"] {
        if let Some(sdk) = std::env::var_os(var) {
            v.push(PathBuf::from(sdk).join("SimConnect SDK").join("lib").join("SimConnect.dll"));
        }
    }
    v.push(PathBuf::from(r"C:\MSFS 2024 SDK\SimConnect SDK\lib\SimConnect.dll"));
    v.push(PathBuf::from(r"C:\MSFS SDK\SimConnect SDK\lib\SimConnect.dll"));
    v
}

impl Api {
    fn load() -> Result<Api, String> {
        let path = dll_candidates()
            .into_iter()
            .find(|p| p.is_file())
            .ok_or("SimConnect.dll not found (put it next to cabin-announcer.exe)")?;
        // SAFETY: SimConnect.dll is a plain Win32 DLL; the symbols below have the
        // signatures declared in SimConnect.h.
        unsafe {
            let lib = Library::new(&path).map_err(|e| format!("loading {}: {e}", path.display()))?;
            macro_rules! sym {
                ($name:literal) => {
                    *lib.get(concat!($name, "\0").as_bytes()).map_err(|e| format!("{}: {e}", $name))?
                };
            }
            Ok(Api {
                open: sym!("SimConnect_Open"),
                close: sym!("SimConnect_Close"),
                add_to_data_definition: sym!("SimConnect_AddToDataDefinition"),
                request_data_on_sim_object: sym!("SimConnect_RequestDataOnSimObject"),
                get_next_dispatch: sym!("SimConnect_GetNextDispatch"),
                map_client_data_name_to_id: sym!("SimConnect_MapClientDataNameToID"),
                add_to_client_data_definition: sym!("SimConnect_AddToClientDataDefinition"),
                request_client_data: sym!("SimConnect_RequestClientData"),
                subscribe_to_system_event: sym!("SimConnect_SubscribeToSystemEvent"),
                _lib: lib,
            })
        }
    }
}

/// One message from the sim. Borrowed data is copied out, since SimConnect
/// reuses its buffer on the next dispatch.
#[derive(Debug)]
pub enum Recv {
    Open,
    Quit,
    Exception { code: u32, send_id: u32, index: u32 },
    Event { id: u32, data: u32 },
    ObjectData { request_id: u32, data: Vec<u8> },
    ClientData { request_id: u32, data: Vec<u8> },
    Other(u32),
}

pub struct SimConnect {
    api: Api,
    handle: Handle,
}

// The handle is only used from the thread that owns the SimConnect value.
unsafe impl Send for SimConnect {}

fn check(hr: HResult, what: &str) -> Result<(), String> {
    if hr >= 0 { Ok(()) } else { Err(format!("{what} failed (0x{:08X})", hr as u32)) }
}

fn cstr(s: &str) -> CString {
    CString::new(s).expect("SimConnect strings never contain NUL")
}

fn dword(b: &[u8], at: usize) -> u32 {
    b.get(at..at + 4).map_or(0, |s| u32::from_le_bytes(s.try_into().unwrap()))
}

impl SimConnect {
    /// Fails quickly when the sim isn't running.
    pub fn open(name: &str) -> Result<SimConnect, String> {
        let api = Api::load()?;
        let mut handle: Handle = std::ptr::null_mut();
        let name = cstr(name);
        let hr = unsafe { (api.open)(&mut handle, name.as_ptr(), std::ptr::null_mut(), 0, std::ptr::null_mut(), 0) };
        check(hr, "SimConnect_Open")?;
        Ok(SimConnect { api, handle })
    }

    pub fn add_to_data_definition(&self, define: u32, name: &str, unit: &str, datatype: u32) -> Result<(), String> {
        let (n, u) = (cstr(name), cstr(unit));
        let unit_ptr = if unit.is_empty() { std::ptr::null() } else { u.as_ptr() };
        let hr = unsafe { (self.api.add_to_data_definition)(self.handle, define, n.as_ptr(), unit_ptr, datatype, 0.0, UNUSED) };
        check(hr, name)
    }

    pub fn request_data_on_user(&self, request: u32, define: u32, period: u32) -> Result<(), String> {
        let hr = unsafe {
            (self.api.request_data_on_sim_object)(self.handle, request, define, OBJECT_ID_USER, period, 0, 0, 0, 0)
        };
        check(hr, "SimConnect_RequestDataOnSimObject")
    }

    /// Subscribes to a whole client data area (e.g. a PMDG data struct).
    pub fn request_client_area(&self, name: &str, area: u32, define: u32, request: u32, size: u32) -> Result<(), String> {
        let n = cstr(name);
        unsafe {
            check((self.api.map_client_data_name_to_id)(self.handle, n.as_ptr(), area), "MapClientDataNameToID")?;
            check((self.api.add_to_client_data_definition)(self.handle, define, 0, size, 0.0, UNUSED), "AddToClientDataDefinition")?;
            check(
                (self.api.request_client_data)(
                    self.handle,
                    area,
                    request,
                    define,
                    CLIENT_DATA_PERIOD_ON_SET,
                    CLIENT_DATA_FLAG_CHANGED,
                    0,
                    0,
                    0,
                ),
                "RequestClientData",
            )
        }
    }

    pub fn subscribe_to_system_event(&self, event: u32, name: &str) -> Result<(), String> {
        let n = cstr(name);
        check(unsafe { (self.api.subscribe_to_system_event)(self.handle, event, n.as_ptr()) }, name)
    }

    /// Next queued message, or None when the queue is empty.
    pub fn next(&self) -> Option<Recv> {
        let mut ptr: *const u8 = std::ptr::null();
        let mut len = 0u32;
        let hr = unsafe { (self.api.get_next_dispatch)(self.handle, &mut ptr, &mut len) };
        if hr < 0 || ptr.is_null() || len < 12 {
            return None;
        }
        // SAFETY: SimConnect guarantees `len` readable bytes at `ptr` until the next call.
        let b = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
        let id = dword(b, 8);
        Some(match id {
            RECV_ID_OPEN => Recv::Open,
            RECV_ID_QUIT => Recv::Quit,
            RECV_ID_EXCEPTION => Recv::Exception { code: dword(b, 12), send_id: dword(b, 16), index: dword(b, 20) },
            RECV_ID_EVENT => Recv::Event { id: dword(b, 16), data: dword(b, 20) },
            RECV_ID_SIMOBJECT_DATA | RECV_ID_CLIENT_DATA => {
                let data = b.get(OBJECT_DATA_OFFSET..).unwrap_or_default().to_vec();
                let request_id = dword(b, 12);
                if id == RECV_ID_CLIENT_DATA {
                    Recv::ClientData { request_id, data }
                } else {
                    Recv::ObjectData { request_id, data }
                }
            }
            other => Recv::Other(other),
        })
    }
}

impl Drop for SimConnect {
    fn drop(&mut self) {
        unsafe { (self.api.close)(self.handle) };
    }
}
