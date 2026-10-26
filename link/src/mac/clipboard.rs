//! Portapapeles de macOS (NSPasteboard): texto e imágenes (PNG; las TIFF se convierten).
use crate::share::Clip;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard, NSPasteboardNameDrag, NSPasteboardTypeFileURL, NSPasteboardTypePNG,
                    NSPasteboardTypeString, NSPasteboardTypeTIFF};
use objc2_foundation::{NSData, NSDictionary, NSString, NSURL};
use std::path::PathBuf;

/// Cambia cada vez que alguien escribe en el portapapeles.
pub fn seq() -> u64 { NSPasteboard::generalPasteboard().changeCount() as u64 }

/// Portapapeles de arrastre: cambia cada vez que empieza un arrastre (p. ej. de archivos en el Finder).
pub fn drag_seq() -> i64 { NSPasteboard::pasteboardWithName(unsafe { NSPasteboardNameDrag }).changeCount() as i64 }

/// Archivos que se están arrastrando, si el arrastre empezó después de `since` (un `drag_seq` anterior).
pub fn dragged_files(since: i64) -> Vec<PathBuf> {
    let pb = NSPasteboard::pasteboardWithName(unsafe { NSPasteboardNameDrag });
    if pb.changeCount() as i64 == since { return Vec::new(); }
    let Some(items) = pb.pasteboardItems() else { return Vec::new() };
    items.iter().filter_map(|it| {
        let s = it.stringForType(unsafe { NSPasteboardTypeFileURL })?;
        // El Finder puede dar URLs de referencia (file:///.file/id=…): se convierten en ruta.
        let path = NSURL::URLWithString(&s)?.filePathURL()?.path()?;
        Some(PathBuf::from(path.to_string()))
    }).filter(|p| p.exists()).collect()
}

pub fn get() -> Option<Clip> {
    let pb = NSPasteboard::generalPasteboard();
    if let Some(s) = pb.stringForType(unsafe { NSPasteboardTypeString }) {
        return Some(Clip::Text(s.to_string()));
    }
    if let Some(png) = pb.dataForType(unsafe { NSPasteboardTypePNG }) {
        return Some(Clip::Png(png.to_vec()));
    }
    // Capturas y apps como Vista Previa copian TIFF: se pasa a PNG.
    let tiff = pb.dataForType(unsafe { NSPasteboardTypeTIFF })?;
    let rep = NSBitmapImageRep::imageRepWithData(&tiff)?;
    let props = NSDictionary::<NSString, AnyObject>::new();
    let png = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &props) }?;
    Some(Clip::Png(png.to_vec()))
}

pub fn set(c: &Clip) {
    let pb = NSPasteboard::generalPasteboard();
    pb.clearContents();
    match c {
        Clip::Text(t) => {
            let t = t.replace("\r\n", "\n");
            pb.setString_forType(&NSString::from_str(&t), unsafe { NSPasteboardTypeString });
        }
        Clip::Png(png) => {
            let data = NSData::with_bytes(png);
            pb.setData_forType(Some(&data), unsafe { NSPasteboardTypePNG });
            // Algunas apps sólo leen TIFF.
            if let Some(tiff) = NSBitmapImageRep::imageRepWithData(&data).and_then(|r| r.TIFFRepresentation()) {
                pb.setData_forType(Some(&tiff), unsafe { NSPasteboardTypeTIFF });
            }
        }
    }
}
