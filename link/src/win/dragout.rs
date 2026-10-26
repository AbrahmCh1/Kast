//! Sacar archivos del otro equipo arrastrándolos desde el visor (al escritorio, al Explorador…): arrastre OLE
//! normal de Windows con los archivos que van llegando a una carpeta temporal. Se suelta en cuanto se suelta el
//! botón, y si el destino pide los archivos antes de que estén completos, espera a que lleguen enteros.
use crate::viewer::DragDone;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use windows::Win32::Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, E_FAIL, E_INVALIDARG, HWND, LPARAM, S_OK, WPARAM};
use windows::Win32::System::Com::{FORMATETC, IAdviseSink, IDataObject, IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA, STGMEDIUM};
use windows::Win32::System::Ole::{IDropSource, IDropSource_Impl, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{ILFindLastID, ILFree, SHCreateDataObject, SHDoDragDrop, SHParseDisplayName};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_LBUTTONUP};
use windows::core::{implement, BOOL, HRESULT, HSTRING, Ref, Result};

/// Origen del arrastre: se suelta al soltar el botón (Windows sólo pregunta cuando cambian los botones o el
/// teclado, así que no se puede retrasar aquí) y se cancela con Esc o si el envío ya falló.
#[implement(IDropSource)]
struct Source { done: Arc<DragDone>, dropped: Arc<AtomicBool> }

impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool() || self.done.state() == Some(false) { return DRAGDROP_S_CANCEL; }
        if keys.0 & MK_LBUTTON.0 != 0 { return S_OK; }
        self.dropped.store(true, Ordering::SeqCst);
        DRAGDROP_S_DROP
    }

    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT { DRAGDROP_S_USEDEFAULTCURSORS }
}

/// Los datos del Explorador para esos archivos (rutas, lista de elementos, imagen del arrastre…), salvo que, una
/// vez soltados, pedir su contenido espera a que hayan llegado enteros.
#[implement(IDataObject)]
struct Data { inner: IDataObject, done: Arc<DragDone>, dropped: Arc<AtomicBool> }

impl Data {
    fn ready(&self) -> Result<()> {
        if self.dropped.load(Ordering::SeqCst) {
            while self.done.state().is_none() { std::thread::sleep(Duration::from_millis(30)); }
            if self.done.state() == Some(false) { return Err(E_FAIL.into()); }
        }
        Ok(())
    }
}

impl IDataObject_Impl for Data_Impl {
    fn GetData(&self, fmt: *const FORMATETC) -> Result<STGMEDIUM> {
        self.ready()?;
        unsafe { self.inner.GetData(fmt) }
    }
    fn GetDataHere(&self, fmt: *const FORMATETC, medium: *mut STGMEDIUM) -> Result<()> {
        self.ready()?;
        unsafe { self.inner.GetDataHere(fmt, medium) }
    }
    fn QueryGetData(&self, fmt: *const FORMATETC) -> HRESULT { unsafe { self.inner.QueryGetData(fmt) } }
    fn GetCanonicalFormatEtc(&self, fmt: *const FORMATETC, out: *mut FORMATETC) -> HRESULT {
        unsafe { self.inner.GetCanonicalFormatEtc(fmt, out) }
    }
    fn SetData(&self, fmt: *const FORMATETC, medium: *const STGMEDIUM, release: BOOL) -> Result<()> {
        unsafe { self.inner.SetData(fmt, medium, release.as_bool()) }
    }
    fn EnumFormatEtc(&self, direction: u32) -> Result<IEnumFORMATETC> { unsafe { self.inner.EnumFormatEtc(direction) } }
    fn DAdvise(&self, fmt: *const FORMATETC, advf: u32, sink: Ref<IAdviseSink>) -> Result<u32> {
        unsafe { self.inner.DAdvise(fmt, advf, sink.ok()?) }
    }
    fn DUnadvise(&self, connection: u32) -> Result<()> { unsafe { self.inner.DUnadvise(connection) } }
    fn EnumDAdvise(&self) -> Result<IEnumSTATDATA> { unsafe { self.inner.EnumDAdvise() } }
}

/// Datos del Explorador para `paths` (todos en la misma carpeta y ya creados, aunque aún se estén llenando).
fn data_object(paths: &[PathBuf], done: &Arc<DragDone>, dropped: &Arc<AtomicBool>) -> Result<IDataObject> {
    let parse = |p: &std::path::Path| unsafe {
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        SHParseDisplayName(&HSTRING::from(p.as_os_str()), None, &mut pidl, 0, None).map(|_| pidl)
    };
    let dir = paths.first().and_then(|p| p.parent()).ok_or_else(|| windows::core::Error::from(E_INVALIDARG))?;
    let folder = parse(dir)?;
    let items: Vec<*mut ITEMIDLIST> = paths.iter().filter_map(|p| parse(p).ok()).collect();
    let r = unsafe {
        let children: Vec<*const ITEMIDLIST> = items.iter().map(|&p| ILFindLastID(p) as *const ITEMIDLIST).collect();
        SHCreateDataObject::<_, IDataObject>(Some(folder), Some(&children), None::<&IDataObject>)
    };
    unsafe {
        for p in items { ILFree(Some(p)); }
        ILFree(Some(folder));
    }
    Ok(Data { inner: r?, done: done.clone(), dropped: dropped.clone() }.into())
}

/// Arrastra `paths` desde la ventana `hwnd` (con el botón izquierdo pulsado). Devuelve true si se soltaron en
/// algún sitio.
pub fn run(hwnd: isize, paths: &[PathBuf], done: &Arc<DragDone>) -> Result<bool> {
    let hwnd = HWND(hwnd as _);
    let dropped = Arc::new(AtomicBool::new(false));
    let r = data_object(paths, done, &dropped).and_then(|data| unsafe {
        let source: IDropSource = Source { done: done.clone(), dropped }.into();
        SHDoDragDrop(Some(hwnd), &data, &source, DROPEFFECT_COPY).map(|e| e != DROPEFFECT_NONE)
    });
    // El arrastre se quedó con el «soltar» del botón: se le pasa a la ventana para que winit libere el ratón.
    unsafe { let _ = PostMessageW(Some(hwnd), WM_LBUTTONUP, WPARAM(0), LPARAM(0)); }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Com::{DVASPECT_CONTENT, TYMED_HGLOBAL};
    use windows::Win32::System::Ole::{OleInitialize, ReleaseStgMedium, CF_HDROP};
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

    /// Las rutas que ve el destino al soltar (CF_HDROP), y que pedirlas espera a que lleguen los archivos.
    #[test]
    fn datos_del_arrastre() {
        unsafe { let _ = OleInitialize(None); }
        let dir = std::env::temp_dir().join(format!("kast-dragout-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Carpeta")).unwrap();
        std::fs::write(dir.join("a.txt"), b"").unwrap();
        let paths = [dir.join("a.txt"), dir.join("Carpeta")];
        let (done, dropped) = (Arc::new(DragDone::default()), Arc::new(AtomicBool::new(false)));
        let data = data_object(&paths, &done, &dropped).unwrap();
        let fmt = FORMATETC { cfFormat: CF_HDROP.0, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
        let files = |data: &IDataObject| unsafe {
            let mut m = data.GetData(&fmt).unwrap();
            let h = HDROP(m.u.hGlobal.0);
            let n = DragQueryFileW(h, u32::MAX, None);
            let out: Vec<PathBuf> = (0..n).map(|i| {
                let mut buf = [0u16; 512];
                let len = DragQueryFileW(h, i, Some(&mut buf)) as usize;
                PathBuf::from(String::from_utf16_lossy(&buf[..len]))
            }).collect();
            ReleaseStgMedium(&mut m);
            // Rutas resueltas (la carpeta temporal puede venir con su nombre corto, p. ej. RUNNER~1).
            out.iter().map(|p| std::fs::canonicalize(p).unwrap()).collect::<Vec<_>>()
        };
        let paths_c: Vec<PathBuf> = paths.iter().map(|p| std::fs::canonicalize(p).unwrap()).collect();
        // Antes de soltar no espera (el destino mira los datos mientras se arrastra por encima).
        assert_eq!(files(&data), paths_c);
        // Soltados: espera a que terminen de llegar.
        dropped.store(true, Ordering::SeqCst);
        let d2 = done.clone();
        let t = std::time::Instant::now();
        std::thread::spawn(move || { std::thread::sleep(Duration::from_millis(300)); d2.finish(true); });
        assert_eq!(files(&data), paths_c);
        assert!(t.elapsed() >= Duration::from_millis(250));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
