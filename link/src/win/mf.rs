//! H.264 con Media Foundation (incluido en Windows: sin dependencias ni licencias de códec).
use std::mem::ManuallyDrop;
use windows::Win32::Foundation::VARIANT_BOOL;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Variant::*;
use windows::core::{GUID, Interface, Result};

pub fn startup() -> Result<()> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_FULL)
    }
}

fn pack(a: u32, b: u32) -> u64 { ((a as u64) << 32) | b as u64 }

fn video_type(subtype: &GUID, w: u32, h: u32, fps: u32) -> Result<IMFMediaType> {
    unsafe {
        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        t.SetGUID(&MF_MT_SUBTYPE, subtype)?;
        t.SetUINT64(&MF_MT_FRAME_SIZE, pack(w, h))?;
        t.SetUINT64(&MF_MT_FRAME_RATE, pack(fps, 1))?;
        t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        Ok(t)
    }
}

fn set_codec(mft: &IMFTransform, key: &GUID, value: u32, boolean: bool) {
    let Ok(api) = mft.cast::<ICodecAPI>() else { return };
    let mut v = VARIANT::default();
    unsafe {
        let inner = &mut *v.Anonymous.Anonymous;
        if boolean {
            inner.vt = VT_BOOL;
            inner.Anonymous.boolVal = VARIANT_BOOL(if value != 0 { -1 } else { 0 });
        } else {
            inner.vt = VT_UI4;
            inner.Anonymous.ulVal = value;
        }
        let _ = api.SetValue(key, &v);
    }
}

/// Baja latencia: el decodificador entrega cada fotograma en cuanto lo tiene, sin acumular.
fn low_latency(mft: &IMFTransform) {
    if let Ok(a) = unsafe { mft.GetAttributes() } {
        let _ = unsafe { a.SetUINT32(&MF_LOW_LATENCY, 1) };
    }
    set_codec(mft, &CODECAPI_AVLowLatencyMode, 1, true);
}

fn new_sample(len: u32) -> Result<IMFSample> {
    unsafe {
        let s = MFCreateSample()?;
        let b = MFCreateMemoryBuffer(len)?;
        s.AddBuffer(&b)?;
        Ok(s)
    }
}

fn sample_bytes(s: &IMFSample) -> Result<Vec<u8>> {
    unsafe {
        let b = s.ConvertToContiguousBuffer()?;
        let mut p = std::ptr::null_mut();
        let mut len = 0u32;
        b.Lock(&mut p, None, Some(&mut len))?;
        let v = std::slice::from_raw_parts(p, len as usize).to_vec();
        b.Unlock()?;
        Ok(v)
    }
}

/// Llama a ProcessOutput una vez con una muestra propia. Ok(None) = necesita más entrada.
fn pull(mft: &IMFTransform, size: u32) -> Result<Option<IMFSample>> {
    let info = unsafe { mft.GetOutputStreamInfo(0)? };
    let provides = info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 != 0;
    let sample = if provides { None } else { Some(new_sample(info.cbSize.max(size))?) };
    let mut out = [MFT_OUTPUT_DATA_BUFFER { dwStreamID: 0, pSample: ManuallyDrop::new(sample), dwStatus: 0, pEvents: ManuallyDrop::new(None) }];
    let mut status = 0u32;
    let r = unsafe { mft.ProcessOutput(0, &mut out, &mut status) };
    let sample = ManuallyDrop::into_inner(std::mem::take(&mut out[0].pSample));
    drop(ManuallyDrop::into_inner(std::mem::take(&mut out[0].pEvents)));
    match r {
        Ok(()) => Ok(sample),
        Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => Ok(None),
        Err(e) => Err(e),
    }
}

pub struct Encoder {
    mft: IMFTransform,
    w: u32,
    h: u32,
    fps: u32,
    n: i64,
}

impl Encoder {
    pub fn new(w: u32, h: u32, fps: u32, bitrate: u32) -> Result<Self> {
        unsafe {
            let mft: IMFTransform = CoCreateInstance(&CLSID_MSH264EncoderMFT, None, CLSCTX_INPROC_SERVER)?;
            let out = video_type(&MFVideoFormat_H264, w, h, fps)?;
            out.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)?;
            out.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_Base.0 as u32)?;
            mft.SetOutputType(0, &out, 0)?;
            mft.SetInputType(0, &video_type(&MFVideoFormat_NV12, w, h, fps)?, 0)?;
            low_latency(&mft);
            set_codec(&mft, &CODECAPI_AVEncCommonRateControlMode, eAVEncCommonRateControlMode_CBR.0 as u32, false);
            set_codec(&mft, &CODECAPI_AVEncMPVGOPSize, fps * 2, false);
            mft.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            mft.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
            Ok(Self { mft, w, h, fps, n: 0 })
        }
    }

    /// Codifica un fotograma NV12; devuelve las unidades H.264 (Annex B) producidas.
    /// El siguiente fotograma sale como fotograma clave.
    pub fn force_keyframe(&self) { set_codec(&self.mft, &CODECAPI_AVEncVideoForceKeyFrame, 1, false); }

    pub fn encode(&mut self, nv12: &[u8]) -> Result<Vec<u8>> {
        let len = (self.w * self.h * 3 / 2) as usize;
        let s = new_sample(len as u32)?;
        unsafe {
            let b = s.GetBufferByIndex(0)?;
            let mut p = std::ptr::null_mut();
            b.Lock(&mut p, None, None)?;
            std::ptr::copy_nonoverlapping(nv12.as_ptr(), p, len);
            b.Unlock()?;
            b.SetCurrentLength(len as u32)?;
            let dur = 10_000_000 / self.fps as i64;
            s.SetSampleTime(self.n * dur)?;
            s.SetSampleDuration(dur)?;
            self.n += 1;
            self.mft.ProcessInput(0, &s, 0)?;
        }
        let mut out = Vec::new();
        while let Some(o) = pull(&self.mft, self.w * self.h)? {
            out.extend_from_slice(&sample_bytes(&o)?);
        }
        Ok(out)
    }
}

pub struct Decoded<'a> {
    pub data: &'a [u8],
    pub pitch: usize,
    pub uv_offset: usize,
}

pub struct Decoder {
    mft: IMFTransform,
    w: u32,
    h: u32,
    alloc_h: u32,
    buf: Vec<u8>,
    pitch: usize,
    fps: u32,
    n: i64,
}

impl Decoder {
    pub fn new(w: u32, h: u32, fps: u32) -> Result<Self> {
        unsafe {
            let mft: IMFTransform = CoCreateInstance(&CLSID_MSH264DecoderMFT, None, CLSCTX_INPROC_SERVER)?;
            low_latency(&mft);
            mft.SetInputType(0, &video_type(&MFVideoFormat_H264, w, h, fps)?, 0)?;
            let mut d = Self { mft, w, h, alloc_h: h, buf: Vec::new(), pitch: w as usize, fps: fps.max(1), n: 0 };
            d.choose_nv12()?;
            d.mft.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            d.mft.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
            Ok(d)
        }
    }

    fn choose_nv12(&mut self) -> Result<()> {
        unsafe {
            let mut i = 0;
            while let Ok(t) = self.mft.GetOutputAvailableType(0, i) {
                if t.GetGUID(&MF_MT_SUBTYPE)? == MFVideoFormat_NV12 {
                    self.mft.SetOutputType(0, &t, 0)?;
                    let size = t.GetUINT64(&MF_MT_FRAME_SIZE).unwrap_or(pack(self.w, self.h));
                    self.alloc_h = (size & 0xFFFF_FFFF) as u32;
                    return Ok(());
                }
                i += 1;
            }
        }
        Err(windows::core::Error::new(MF_E_INVALIDMEDIATYPE, "el decodificador no ofrece NV12"))
    }

    /// Decodifica una unidad de acceso H.264. Devuelve el último fotograma disponible y cuántos salieron.
    pub fn decode(&mut self, h264: &[u8]) -> Result<(usize, Option<Decoded<'_>>)> {
        let s = new_sample(h264.len() as u32)?;
        unsafe {
            let b = s.GetBufferByIndex(0)?;
            let mut p = std::ptr::null_mut();
            b.Lock(&mut p, None, None)?;
            std::ptr::copy_nonoverlapping(h264.as_ptr(), p, h264.len());
            b.Unlock()?;
            b.SetCurrentLength(h264.len() as u32)?;
            let dur = 10_000_000 / self.fps as i64;
            s.SetSampleTime(self.n * dur)?;
            s.SetSampleDuration(dur)?;
            self.n += 1;
        }
        let mut got = 0;
        // Si tiene salida pendiente no acepta más entrada: se vacía y se reintenta.
        loop {
            match unsafe { self.mft.ProcessInput(0, &s, 0) } {
                Ok(()) => break,
                Err(e) if e.code() == MF_E_NOTACCEPTING => {
                    if self.drain_one()? { got += 1 } else { return Err(e) }
                }
                Err(e) => return Err(e),
            }
        }
        while self.drain_one()? { got += 1; }
        Ok((got, (got > 0).then(|| Decoded { data: &self.buf, pitch: self.pitch, uv_offset: self.pitch * self.alloc_h as usize })))
    }

    /// Saca un fotograma decodificado (si hay) a `self.buf`.
    fn drain_one(&mut self) -> Result<bool> {
        loop {
            let frame = match pull(&self.mft, self.w * self.alloc_h * 3 / 2) {
                Ok(Some(f)) => f,
                Ok(None) => return Ok(false),
                Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    self.choose_nv12()?;
                    continue;
                }
                Err(e) => return Err(e),
            };
            unsafe {
                let b = frame.GetBufferByIndex(0)?;
                if let Ok(b2) = b.cast::<IMF2DBuffer>() {
                    let mut p = std::ptr::null_mut();
                    let mut pitch = 0i32;
                    b2.Lock2D(&mut p, &mut pitch)?;
                    let len = pitch as usize * self.alloc_h as usize * 3 / 2;
                    self.buf.resize(len, 0);
                    std::ptr::copy_nonoverlapping(p, self.buf.as_mut_ptr(), len);
                    b2.Unlock2D()?;
                    self.pitch = pitch as usize;
                } else {
                    self.buf = sample_bytes(&frame)?;
                    self.pitch = self.w as usize;
                }
            }
            return Ok(true);
        }
    }
}
