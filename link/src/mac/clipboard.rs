//! Portapapeles de macOS (NSPasteboard): texto e imágenes (PNG; las TIFF se convierten).
use crate::share::Clip;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard, NSPasteboardTypePNG, NSPasteboardTypeString, NSPasteboardTypeTIFF};
use objc2_foundation::{NSData, NSDictionary, NSString};

/// Cambia cada vez que alguien escribe en el portapapeles.
pub fn seq() -> u64 { NSPasteboard::generalPasteboard().changeCount() as u64 }

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
