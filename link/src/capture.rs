//! Captura de un monitor con DXGI Desktop Duplication (Windows 8+).
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::core::{Interface, Result};

pub struct Output {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    adapter: IDXGIAdapter1,
    output: IDXGIOutput,
}

/// Monitores conectados al escritorio, de izquierda a derecha (mismo orden que Kast).
pub fn outputs() -> Result<Vec<Output>> {
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1()? };
    let mut list = Vec::new();
    let mut ai = 0;
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(ai) } {
        let mut oi = 0;
        while let Ok(output) = unsafe { adapter.EnumOutputs(oi) } {
            let d = unsafe { output.GetDesc()? };
            if d.AttachedToDesktop.as_bool() {
                let n = d.DeviceName.iter().position(|&c| c == 0).unwrap_or(32);
                let r = d.DesktopCoordinates;
                list.push(Output {
                    name: String::from_utf16_lossy(&d.DeviceName[..n]),
                    x: r.left,
                    y: r.top,
                    w: (r.right - r.left) as u32,
                    h: (r.bottom - r.top) as u32,
                    adapter: adapter.clone(),
                    output,
                });
            }
            oi += 1;
        }
        ai += 1;
    }
    list.sort_by_key(|o| (o.x, o.y));
    Ok(list)
}

pub struct Frame<'a> {
    pub data: &'a [u8],
    pub pitch: usize,
}

pub struct Capturer {
    ctx: ID3D11DeviceContext,
    device: ID3D11Device,
    output: IDXGIOutput1,
    dup: IDXGIOutputDuplication,
    staging: Option<ID3D11Texture2D>,
    pub w: u32,
    pub h: u32,
    pub cursor: (i32, i32, bool),
    buf: Vec<u8>,
    pitch: usize,
}

impl Capturer {
    pub fn new(o: &Output) -> Result<Self> {
        let mut device = None;
        let mut ctx = None;
        unsafe {
            D3D11CreateDevice(&o.adapter, D3D_DRIVER_TYPE_UNKNOWN, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                              None, D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut ctx))?;
        }
        let device = device.unwrap();
        let output: IDXGIOutput1 = o.output.cast()?;
        let dup = unsafe { output.DuplicateOutput(&device)? };
        Ok(Self { ctx: ctx.unwrap(), device, output, dup, staging: None, w: o.w, h: o.h, cursor: (0, 0, false), buf: Vec::new(), pitch: 0 })
    }

    /// Espera hasta `timeout_ms` un fotograma nuevo. Devuelve `None` si la imagen no cambió
    /// (aunque el cursor puede haberse movido: ver `self.cursor`).
    pub fn next(&mut self, timeout_ms: u32) -> Result<Option<Frame<'_>>> {
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut res: Option<IDXGIResource> = None;
        match unsafe { self.dup.AcquireNextFrame(timeout_ms, &mut info, &mut res) } {
            Ok(()) => {}
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(None),
            Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                // Cambio de modo, UAC, pantalla de bloqueo...: se recrea la duplicación.
                self.dup = unsafe { self.output.DuplicateOutput(&self.device)? };
                return Ok(None);
            }
            Err(e) => return Err(e),
        }
        if info.LastMouseUpdateTime != 0 {
            let p = info.PointerPosition;
            self.cursor = (p.Position.x, p.Position.y, p.Visible.as_bool());
        }
        let fresh = info.AccumulatedFrames > 0 || info.LastPresentTime != 0;
        if fresh {
            let tex: ID3D11Texture2D = res.unwrap().cast()?;
            if self.staging.is_none() {
                let mut desc = D3D11_TEXTURE2D_DESC::default();
                unsafe { tex.GetDesc(&mut desc) };
                self.w = desc.Width;
                self.h = desc.Height;
                desc.Usage = D3D11_USAGE_STAGING;
                desc.BindFlags = 0;
                desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
                desc.MiscFlags = 0;
                let mut st = None;
                unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut st))? };
                self.staging = st;
            }
            let st = self.staging.as_ref().unwrap();
            unsafe { self.ctx.CopyResource(st, &tex) };
        }
        unsafe { self.dup.ReleaseFrame()? };
        if !fresh {
            return Ok(None);
        }
        let st = self.staging.as_ref().unwrap();
        let mut map = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe { self.ctx.Map(st, 0, D3D11_MAP_READ, 0, Some(&mut map))? };
        self.pitch = map.RowPitch as usize;
        let len = self.pitch * self.h as usize;
        self.buf.resize(len, 0);
        unsafe { std::ptr::copy_nonoverlapping(map.pData as *const u8, self.buf.as_mut_ptr(), len) };
        unsafe { self.ctx.Unmap(st, 0) };
        Ok(Some(Frame { data: &self.buf, pitch: self.pitch }))
    }
}
