//! Plataforma macOS: captura con ScreenCaptureKit y H.264 por hardware con VideoToolbox.
//! ScreenCaptureKit entrega NV12 en memoria de la GPU (IOSurface) que pasa directo al codificador:
//! no hay conversión de color en CPU. VideoToolbox usa AVCC; se convierte a/desde Annex B (h264.rs).
use crate::h264::{self, NAL_PPS, NAL_SPS};
use crate::{DisplayInfo, Encoded, Res, now_us};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
use objc2_core_foundation::{CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType, kCFBooleanFalse, kCFBooleanTrue};
use objc2_core_graphics::{CGDirectDisplayID, CGDisplayBounds, CGDisplayCopyDisplayMode, CGDisplayMode, CGGetActiveDisplayList};
use objc2_core_media::{CMBlockBuffer, CMFormatDescription, CMSampleBuffer, CMTime, CMVideoFormatDescriptionCreateFromH264ParameterSets,
                       CMVideoFormatDescriptionGetH264ParameterSetAtIndex, kCMVideoCodecType_H264};
use objc2_core_video::{CVImageBuffer, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight,
                       CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
                       kCVPixelBufferPixelFormatTypeKey, kCVPixelFormatType_32BGRA, kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange};
use objc2_foundation::{NSArray, NSError};
use objc2_screen_capture_kit::{SCContentFilter, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamOutput, SCStreamOutputType};
use objc2_video_toolbox::*;
use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::{NonNull, null, null_mut};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

mod input;
pub use input::{Injector, input_allowed, request_input};

pub fn init() {}

const NO_PERMISSION: &str = "Kast no tiene permiso de Grabación de pantalla en este Mac \
    (Ajustes del Sistema → Privacidad y seguridad → Grabación de pantalla). Actívalo y vuelve a abrir Kast";

/// Some(false) si falta el permiso de Grabación de pantalla (macOS lo exige para compartir).
pub fn capture_allowed() -> Option<bool> { Some(objc2_core_graphics::CGPreflightScreenCaptureAccess()) }

/// Pide el permiso (macOS muestra su aviso la primera vez) y abre su panel en Ajustes del Sistema.
pub fn request_capture() {
    if !objc2_core_graphics::CGRequestScreenCaptureAccess() {
        let _ = std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
            .spawn();
    }
}

fn active_displays() -> Vec<CGDirectDisplayID> {
    let mut ids = [0 as CGDirectDisplayID; 16];
    let mut n = 0u32;
    unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut n) };
    let mut v: Vec<_> = ids[..n as usize].to_vec();
    v.sort_by(|a, b| {
        let (ra, rb) = (CGDisplayBounds(*a), CGDisplayBounds(*b));
        (ra.origin.x, ra.origin.y).partial_cmp(&(rb.origin.x, rb.origin.y)).unwrap()
    });
    v
}

/// Tamaño en píxeles reales (no puntos) del modo actual.
fn pixel_size(id: CGDirectDisplayID) -> (u32, u32) {
    let mode = CGDisplayCopyDisplayMode(id);
    let w = CGDisplayMode::pixel_width(mode.as_deref());
    let h = CGDisplayMode::pixel_height(mode.as_deref());
    (w as u32, h as u32)
}

pub fn displays() -> Res<Vec<DisplayInfo>> {
    Ok(active_displays()
        .into_iter()
        .map(|id| {
            let b = CGDisplayBounds(id);
            let (w, h) = pixel_size(id);
            DisplayInfo { name: format!("ID {id}"), x: b.origin.x as i32, y: b.origin.y as i32, w, h }
        })
        .collect())
}

fn ns_error(e: *mut NSError) -> String {
    if e.is_null() { return "error desconocido".into(); }
    unsafe { (*e).localizedDescription().to_string() }
}

fn cf_bool(v: bool) -> &'static CFBoolean { unsafe { if v { kCFBooleanTrue } else { kCFBooleanFalse }.unwrap() } }

fn set_prop(s: &VTCompressionSession, key: &CFString, value: &CFType) {
    unsafe { VTSessionSetProperty(s, key, Some(value)) };
}

// ---------------------------------------------------------------- codificador

/// Estado que comparte el callback del codificador (vive mientras vive la sesión).
struct EncCtx {
    tx: Mutex<Sender<Encoded>>,
}

unsafe extern "C-unwind" fn enc_callback(refcon: *mut c_void, src: *mut c_void, status: i32, _flags: VTEncodeInfoFlags, sbuf: *mut CMSampleBuffer) {
    if status != 0 || sbuf.is_null() || refcon.is_null() { return; }
    let ctx = unsafe { &*(refcon as *const EncCtx) };
    let sbuf = unsafe { &*sbuf };
    let Some(bb) = (unsafe { sbuf.data_buffer() }) else { return };
    let len = unsafe { bb.data_length() };
    let mut avcc = vec![0u8; len];
    if unsafe { bb.copy_data_bytes(0, len, NonNull::new(avcc.as_mut_ptr().cast()).unwrap()) } != 0 { return; }
    let mut out = Vec::with_capacity(len + 128);
    // En los fotogramas clave se anteponen SPS y PPS (VideoToolbox los guarda en la descripción de formato).
    let mut test = Vec::new();
    if h264::avcc_to_annexb(&avcc, &mut test) {
        if let Some(desc) = unsafe { sbuf.format_description() } {
            for i in 0..2 {
                let (mut p, mut sz) = (null::<u8>(), 0usize);
                let st = unsafe { CMVideoFormatDescriptionGetH264ParameterSetAtIndex(&desc, i, &mut p, &mut sz, null_mut(), null_mut()) };
                if st == 0 && !p.is_null() {
                    let ps = unsafe { std::slice::from_raw_parts(p, sz) };
                    // VideoToolbox no declara "0 fotogramas reordenados": sin eso el decodificador de Windows
                    // retiene ~5 fotogramas. Se corrige el SPS antes de enviarlo.
                    match h264::sps_low_latency(ps) {
                        Some(fixed) => h264::push_annexb(&mut out, &fixed),
                        None => h264::push_annexb(&mut out, ps),
                    }
                }
            }
        }
    }
    out.extend_from_slice(&test);
    let ts = src as u64;
    let _ = ctx.tx.lock().unwrap().send(Encoded { data: out, ts_us: ts, work_ms: now_us().saturating_sub(ts) as f32 / 1000.0 });
}

struct OutputIvars {
    session: CFRetained<VTCompressionSession>,
    fps: i32,
    first: Cell<bool>,
}

define_class!(
    // Recibe los fotogramas de ScreenCaptureKit y los pasa al codificador.
    #[unsafe(super(NSObject))]
    #[name = "KastLinkStreamOutput"]
    #[ivars = OutputIvars]
    struct StreamOutput;

    unsafe impl NSObjectProtocol for StreamOutput {}

    unsafe impl SCStreamOutput for StreamOutput {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn did_output(&self, _stream: &SCStream, sample: &CMSampleBuffer, kind: SCStreamOutputType) {
            if kind != SCStreamOutputType::Screen { return; }
            // Los fotogramas "sin cambios" no traen imagen.
            let Some(img) = (unsafe { sample.image_buffer() }) else { return };
            let iv = self.ivars();
            let ts = now_us();
            let pts = unsafe { sample.presentation_time_stamp() };
            let dur = unsafe { CMTime::new(1, iv.fps) };
            let props = if iv.first.replace(false) {
                let key: &CFString = unsafe { kVTEncodeFrameOptionKey_ForceKeyFrame };
                Some(CFDictionary::<CFString, CFBoolean>::from_slices(&[key], &[cf_bool(true)]))
            } else {
                None
            };
            unsafe {
                iv.session.encode_frame(&img, pts, dur, props.as_deref().map(|d| d.as_opaque()), ts as *mut c_void, null_mut());
            }
        }
    }
);

impl StreamOutput {
    fn new(ivars: OutputIvars) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ivars);
        unsafe { msg_send![super(this), init] }
    }
}

pub struct Source {
    _stream: Retained<SCStream>,
    _output: Retained<StreamOutput>,
    _ctx: Box<EncCtx>,
    rx: Receiver<Encoded>,
    w: u32,
    h: u32,
    wait: Duration,
}

/// Puntero que cruza al hilo de la llamada de retorno de ScreenCaptureKit.
struct SendPtr(*mut SCShareableContent);
unsafe impl Send for SendPtr {}

fn shareable_content() -> Res<Retained<SCShareableContent>> {
    let (tx, rx) = channel::<Result<SendPtr, String>>();
    let tx = Mutex::new(tx);
    let block = RcBlock::new(move |content: *mut SCShareableContent, err: *mut NSError| {
        let r = if content.is_null() { Err(ns_error(err)) } else {
            // Se retiene aquí; el receptor toma posesión.
            Ok(SendPtr(Retained::into_raw(unsafe { Retained::retain(content) }.unwrap())))
        };
        let _ = tx.lock().unwrap().send(r);
    });
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&block) };
    let r = rx.recv_timeout(Duration::from_secs(15)).map_err(|_| "ScreenCaptureKit no respondió")?;
    let p = r.map_err(|e| format!("{NO_PERMISSION} ({e})"))?;
    Ok(unsafe { Retained::from_raw(p.0) }.unwrap())
}

impl Source {
    /// show_cursor: dibujar el cursor en la imagen (no si el otro equipo controla con su propio cursor).
    pub fn new(index: usize, fps: u32, mbps: u32, use_pattern: bool, show_cursor: bool) -> Res<Self> {
        if use_pattern { return Err("--pattern sólo está disponible en Windows".into()); }
        if capture_allowed() == Some(false) { return Err(NO_PERMISSION.into()); }
        let ids = active_displays();
        let id = *ids.get(index.wrapping_sub(1)).ok_or_else(|| format!("monitor {index} no existe (hay {})", ids.len()))?;
        let (pw, ph) = pixel_size(id);
        let scale = (3840.0 / pw as f64).min(1.0);
        let (w, h) = (((pw as f64 * scale) as u32) & !1, ((ph as f64 * scale) as u32) & !1);

        let content = shareable_content()?;
        let displays = unsafe { content.displays() };
        let display = displays.iter().find(|d| unsafe { d.displayID() } == id).ok_or("ScreenCaptureKit no encuentra el monitor")?;

        // Codificador H.264 en tiempo real, sin fotogramas B, perfil Baseline.
        let (tx, rx) = channel();
        let ctx = Box::new(EncCtx { tx: Mutex::new(tx) });
        let mut raw: *mut VTCompressionSession = null_mut();
        let st = unsafe {
            VTCompressionSession::create(None, w as i32, h as i32, kCMVideoCodecType_H264, None, None, None, Some(enc_callback),
                                         &*ctx as *const EncCtx as *mut c_void, NonNull::from(&mut raw))
        };
        if st != 0 || raw.is_null() { return Err(format!("VideoToolbox no pudo crear el codificador ({st})").into()); }
        let session = unsafe { CFRetained::from_raw(NonNull::new(raw).unwrap()) };
        unsafe {
            set_prop(&session, kVTCompressionPropertyKey_RealTime, cf_bool(true));
            set_prop(&session, kVTCompressionPropertyKey_AllowFrameReordering, cf_bool(false));
            set_prop(&session, kVTCompressionPropertyKey_ProfileLevel, kVTProfileLevel_H264_Baseline_AutoLevel);
            set_prop(&session, kVTCompressionPropertyKey_AverageBitRate, &CFNumber::new_i32((mbps * 1_000_000) as i32));
            set_prop(&session, kVTCompressionPropertyKey_MaxKeyFrameInterval, &CFNumber::new_i32((fps * 2) as i32));
            set_prop(&session, kVTCompressionPropertyKey_ExpectedFrameRate, &CFNumber::new_i32(fps as i32));
            session.prepare_to_encode_frames();
        }

        // ScreenCaptureKit: NV12 de rango limitado BT.709 (lo que espera el decodificador de Windows), con el cursor.
        let filter = unsafe { SCContentFilter::initWithDisplay_excludingWindows(SCContentFilter::alloc(), &display, &NSArray::new()) };
        let config = unsafe { SCStreamConfiguration::new() };
        unsafe {
            config.setWidth(w as usize);
            config.setHeight(h as usize);
            config.setPixelFormat(kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange);
            config.setMinimumFrameInterval(CMTime::new(1, fps as i32));
            config.setShowsCursor(show_cursor);
            config.setQueueDepth(5);
            config.setColorMatrix(objc2_core_graphics::kCGDisplayStreamYCbCrMatrix_ITU_R_709_2);
        }
        let stream = unsafe { SCStream::initWithFilter_configuration_delegate(SCStream::alloc(), &filter, &config, None) };
        let output = StreamOutput::new(OutputIvars { session, fps: fps as i32, first: Cell::new(true) });
        let queue = dispatch2::DispatchQueue::new("io.github.abrahmch1.kast-link.capture", None);
        unsafe { stream.addStreamOutput_type_sampleHandlerQueue_error(ProtocolObject::from_ref(&*output), SCStreamOutputType::Screen, Some(&queue)) }
            .map_err(|e| format!("ScreenCaptureKit: {}", e.localizedDescription()))?;
        let (stx, srx) = channel::<Option<String>>();
        let stx = Mutex::new(stx);
        let done = RcBlock::new(move |err: *mut NSError| { let _ = stx.lock().unwrap().send((!err.is_null()).then(|| ns_error(err))); });
        unsafe { stream.startCaptureWithCompletionHandler(Some(&done)) };
        if let Some(e) = srx.recv_timeout(Duration::from_secs(15)).map_err(|_| "ScreenCaptureKit no arrancó")? {
            return Err(format!("no se pudo capturar la pantalla (¿permiso de grabación de pantalla?): {e}").into());
        }
        Ok(Self { _stream: stream, _output: output, _ctx: ctx, rx, w, h, wait: Duration::from_millis((2000 / fps.max(1)) as u64 + 5) })
    }

    pub fn size(&self) -> (u32, u32) { (self.w, self.h) }

    /// Detiene la captura y el codificador (sin esto ScreenCaptureKit seguía capturando y VideoToolbox llamaba a
    /// `enc_callback` con el contexto ya liberado: el agente se caía al terminar una sesión).
    fn stop(&self) {
        let (tx, rx) = channel::<()>();
        let tx = Mutex::new(tx);
        let done = RcBlock::new(move |_err: *mut NSError| { let _ = tx.lock().unwrap().send(()); });
        unsafe { self._stream.stopCaptureWithCompletionHandler(Some(&done)) };
        let _ = rx.recv_timeout(Duration::from_secs(3));
        let session = &self._output.ivars().session;
        unsafe {
            session.complete_frames(objc2_core_media::kCMTimeInvalid);
            session.invalidate();
        }
    }

    /// ScreenCaptureKit dibuja el cursor dentro de la imagen.
    pub fn cursor(&self) -> (i32, i32, bool) { (0, 0, false) }

    pub fn next(&mut self) -> Res<Option<Encoded>> {
        match self.rx.recv_timeout(self.wait) {
            Ok(e) => Ok(Some(e)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err("el codificador se detuvo".into()),
        }
    }
}

impl Drop for Source {
    // Antes de liberar `_ctx` (refcon del codificador): después de esto ya no llegan fotogramas ni callbacks.
    fn drop(&mut self) { self.stop(); }
}

// ---------------------------------------------------------------- decodificador

struct DecOut {
    px: Vec<u32>,
    w: usize,
    h: usize,
    frames: usize,
}

unsafe extern "C-unwind" fn dec_callback(refcon: *mut c_void, _src: *mut c_void, status: i32, _flags: VTDecodeInfoFlags,
                                         image: *mut CVImageBuffer, _pts: CMTime, _dur: CMTime) {
    if status != 0 || image.is_null() || refcon.is_null() { return; }
    let out = unsafe { &mut *(refcon as *mut DecOut) };
    let pb = unsafe { &*image };
    unsafe { CVPixelBufferLockBaseAddress(pb, CVPixelBufferLockFlags::ReadOnly) };
    let base = CVPixelBufferGetBaseAddress(pb) as *const u8;
    let stride = CVPixelBufferGetBytesPerRow(pb);
    let (w, h) = (CVPixelBufferGetWidth(pb), CVPixelBufferGetHeight(pb));
    if !base.is_null() {
        out.px.resize(w * h, 0);
        for y in 0..h {
            let row = unsafe { std::slice::from_raw_parts(base.add(y * stride) as *const u32, w) };
            for (d, s) in out.px[y * w..(y + 1) * w].iter_mut().zip(row) { *d = u32::from_le(*s) & 0x00FF_FFFF; }
        }
        out.w = w;
        out.h = h;
        out.frames += 1;
    }
    unsafe { CVPixelBufferUnlockBaseAddress(pb, CVPixelBufferLockFlags::ReadOnly) };
}

pub struct Decoder {
    session: Option<CFRetained<VTDecompressionSession>>,
    format: Option<CFRetained<CMFormatDescription>>,
    sps: Vec<u8>,
    pps: Vec<u8>,
    out: Box<DecOut>,
    w: usize,
    h: usize,
}

impl Decoder {
    pub fn new(w: u32, h: u32, _fps: u32) -> Res<Self> {
        Ok(Self { session: None, format: None, sps: Vec::new(), pps: Vec::new(),
                  out: Box::new(DecOut { px: Vec::new(), w: 0, h: 0, frames: 0 }), w: w as usize, h: h as usize })
    }

    fn rebuild(&mut self) -> Res<()> {
        if let Some(s) = self.session.take() { unsafe { s.invalidate() }; }
        let ptrs = [NonNull::new(self.sps.as_ptr() as *mut u8).unwrap(), NonNull::new(self.pps.as_ptr() as *mut u8).unwrap()];
        let sizes = [self.sps.len(), self.pps.len()];
        let mut fmt: *const CMFormatDescription = null();
        let st = unsafe {
            CMVideoFormatDescriptionCreateFromH264ParameterSets(None, 2, NonNull::from(&ptrs[0]).cast(), NonNull::from(&sizes[0]), 4, NonNull::from(&mut fmt))
        };
        if st != 0 || fmt.is_null() { return Err(format!("parámetros H.264 inválidos ({st})").into()); }
        let format = unsafe { CFRetained::from_raw(NonNull::new(fmt as *mut CMFormatDescription).unwrap()) };
        let key: &CFString = unsafe { kCVPixelBufferPixelFormatTypeKey };
        let bgra = CFNumber::new_i32(kCVPixelFormatType_32BGRA as i32);
        let attrs = CFDictionary::<CFString, CFNumber>::from_slices(&[key], &[&*bgra]);
        let record = VTDecompressionOutputCallbackRecord {
            decompressionOutputCallback: Some(dec_callback),
            decompressionOutputRefCon: &mut *self.out as *mut DecOut as *mut c_void,
        };
        let mut raw: *mut VTDecompressionSession = null_mut();
        let st = unsafe { VTDecompressionSession::create(None, &format, None, Some(attrs.as_opaque()), &record, NonNull::from(&mut raw)) };
        if st != 0 || raw.is_null() { return Err(format!("VideoToolbox no pudo crear el decodificador ({st})").into()); }
        self.session = Some(unsafe { CFRetained::from_raw(NonNull::new(raw).unwrap()) });
        self.format = Some(format);
        Ok(())
    }

    /// Decodifica una unidad de acceso Annex B; escribe el último fotograma en `out` (0RGB).
    pub fn decode(&mut self, data: &[u8], out: &mut [u32]) -> Res<usize> {
        let nals = h264::nal_units(data);
        let (mut sps, mut pps) = (None, None);
        for n in &nals {
            match h264::nal_type(n) { NAL_SPS => sps = Some(*n), NAL_PPS => pps = Some(*n), _ => {} }
        }
        if let (Some(s), Some(p)) = (sps, pps) {
            if self.session.is_none() || s != self.sps.as_slice() || p != self.pps.as_slice() {
                self.sps = s.to_vec();
                self.pps = p.to_vec();
                self.rebuild()?;
            }
        }
        let (Some(session), Some(format)) = (&self.session, &self.format) else { return Ok(0) }; // espera un fotograma clave
        let avcc = h264::to_avcc(nals.into_iter().filter(|n| !matches!(h264::nal_type(n), NAL_SPS | NAL_PPS | 9)));
        if avcc.is_empty() { return Ok(0); }

        let mut bb: *mut CMBlockBuffer = null_mut();
        let st = unsafe { CMBlockBuffer::create_with_memory_block(None, null_mut(), avcc.len(), None, null(), 0, avcc.len(), 1, NonNull::from(&mut bb)) };
        if st != 0 || bb.is_null() { return Err(format!("CMBlockBuffer ({st})").into()); }
        let bb = unsafe { CFRetained::from_raw(NonNull::new(bb).unwrap()) };
        unsafe { CMBlockBuffer::replace_data_bytes(NonNull::new(avcc.as_ptr() as *mut c_void).unwrap(), &bb, 0, avcc.len()) };
        let sizes = [avcc.len()];
        let mut sb: *mut CMSampleBuffer = null_mut();
        let st = unsafe { CMSampleBuffer::create_ready(None, Some(&bb), Some(format), 1, 0, null(), 1, sizes.as_ptr(), NonNull::from(&mut sb)) };
        if st != 0 || sb.is_null() { return Err(format!("CMSampleBuffer ({st})").into()); }
        let sb = unsafe { CFRetained::from_raw(NonNull::new(sb).unwrap()) };

        self.out.frames = 0;
        // Sin la opción asíncrona, la llamada de retorno se ejecuta antes de volver.
        let st = unsafe { session.decode_frame(&sb, VTDecodeFrameFlags(0), null_mut(), null_mut()) };
        if st != 0 { return Err(format!("VideoToolbox no pudo decodificar ({st})").into()); }
        if self.out.frames > 0 {
            let (w, h) = (self.out.w.min(self.w), self.out.h.min(self.h));
            for y in 0..h {
                out[y * self.w..y * self.w + w].copy_from_slice(&self.out.px[y * self.out.w..y * self.out.w + w]);
            }
        }
        Ok(self.out.frames)
    }
}
