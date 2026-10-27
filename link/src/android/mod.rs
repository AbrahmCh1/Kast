//! Puente JNI con la app de Android (android/, clase io.github.abrahmch1.kast.Native). El cliente (ver y controlar
//! otros equipos) es el de mobile.rs; en pantallas grandes, además, el anfitrión (host.rs) comparte esta pantalla.
//! Los eventos llegan a Kotlin como JSON (Native.onEvent) y el vídeo de las sesiones en Annex B (Native.onFrame).
mod host;

use crate::input::Event;
use crate::net::msg;
use crate::{mobile, store};
use host::Host;
use jni::JNIEnv;
use jni::JavaVM;
use jni::objects::{GlobalRef, JByteBuffer, JClass, JString, JValue};
use jni::sys::{JNI_FALSE, JNI_TRUE, jboolean, jint, jlong};
use serde_json::Value;
use std::ffi::{CStr, c_char, c_void};
use std::sync::{Arc, OnceLock};

static VM: OnceLock<JavaVM> = OnceLock::new();
static CLASS: OnceLock<GlobalRef> = OnceLock::new();
static HOST: OnceLock<Arc<Host>> = OnceLock::new();

/// Llama a Kotlin desde cualquier hilo (el hilo queda asociado a la máquina virtual).
fn java(f: impl FnOnce(&mut JNIEnv, &JClass) -> jni::errors::Result<()>) {
    let (Some(vm), Some(cls)) = (VM.get(), CLASS.get()) else { return };
    let Ok(mut env) = vm.attach_current_thread_permanently() else { return };
    let cls: &JClass = cls.as_obj().into();
    let r = env.with_local_frame(8, |env| f(env, cls));
    if r.is_err() && env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
}

fn emit_str(json: &str) {
    java(|env, cls| {
        let s = env.new_string(json)?;
        env.call_static_method(cls, "onEvent", "(Ljava/lang/String;)V", &[JValue::Object(&s)])?;
        Ok(())
    });
}

fn emit(v: &Value) { emit_str(&v.to_string()) }

extern "C" fn on_event(_ctx: *mut c_void, json: *const c_char) {
    if json.is_null() { return; }
    emit_str(&unsafe { CStr::from_ptr(json) }.to_string_lossy());
}

/// Fotograma de una sesión (Annex B; `sps_len` > 0 en los fotogramas clave).
extern "C" fn on_frame(_ctx: *mut c_void, session: u64, data: *const u8, len: usize, _sps: *const u8, sps_len: usize, _pps: *const u8, _pps_len: usize) {
    let data = unsafe { std::slice::from_raw_parts(data, len) };
    java(|env, cls| {
        let arr = env.byte_array_from_slice(data)?;
        env.call_static_method(cls, "onFrame", "(J[BZ)V", &[JValue::Long(session as i64), JValue::Object(&arr), JValue::Bool((sps_len > 0) as u8)])?;
        Ok(())
    });
}

/// Registro en logcat (etiqueta «Kast»).
pub fn logcat(msg: &str) {
    #[link(name = "log")]
    unsafe extern "C" {
        fn __android_log_write(prio: i32, tag: *const c_char, text: *const c_char) -> i32;
    }
    if let Ok(t) = std::ffi::CString::new(msg.replace('\0', " ")) {
        unsafe { __android_log_write(4, c"Kast".as_ptr(), t.as_ptr()) };
    }
}

fn string(env: &mut JNIEnv, s: &JString) -> String {
    if s.is_null() { return String::new(); }
    env.get_string(s).map(String::from).unwrap_or_default()
}

// ---------------------------------------------------------------- métodos nativos de Native.kt

/// Arranca el cliente y, con `host`, el anfitrión (pantallas grandes). `dir`: carpeta de datos de la app.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_abrahmch1_kast_Native_start(mut env: JNIEnv, class: JClass, dir: JString, name: JString, host: jboolean) -> jboolean {
    if VM.get().is_none() {
        let Ok(vm) = env.get_java_vm() else { return JNI_FALSE };
        let Ok(cls) = env.new_global_ref(&class) else { return JNI_FALSE };
        let _ = VM.set(vm);
        let _ = CLASS.set(cls);
    }
    let (dir, name) = (string(&mut env, &dir), string(&mut env, &name));
    if !mobile::start(dir, name.clone(), std::ptr::null_mut(), on_event, on_frame, true) { return JNI_FALSE; }
    if host != JNI_FALSE && HOST.get().is_none() {
        match Host::start(name, emit) {
            Ok(h) => { let _ = HOST.set(h); }
            Err(e) => store::log(&format!("no se pudo compartir esta pantalla: {e}")),
        }
    }
    JNI_TRUE
}

/// Comandos en JSON del anfitrión (ver host.rs) o del cliente (ver mobile.rs).
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_abrahmch1_kast_Native_command(mut env: JNIEnv, _: JClass, json: JString) {
    let json = string(&mut env, &json);
    let Ok(v) = serde_json::from_str::<Value>(&json) else { return };
    if HOST.get().is_some_and(|h| h.command(&v)) { return; }
    mobile::command(&json);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_abrahmch1_kast_Native_connect(mut env: JNIEnv, _: JClass, id: JString, monitor: jint, control: jboolean,
                                                                     auth: JString, secret: JString) -> jlong {
    let (id, auth, secret) = (string(&mut env, &id), string(&mut env, &auth), string(&mut env, &secret));
    mobile::connect(&id, monitor.max(1) as u32, control != JNI_FALSE, (!auth.is_empty()).then_some((auth, secret))) as jlong
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_abrahmch1_kast_Native_keyframe(_: JNIEnv, _: JClass, session: jlong) {
    mobile::send(session as u64, vec![msg::KEYFRAME]);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_abrahmch1_kast_Native_mouseMove(_: JNIEnv, _: JClass, session: jlong, x: jint, y: jint) {
    mobile::send(session as u64, Event::Move { x, y }.encode());
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_abrahmch1_kast_Native_mouseButton(_: JNIEnv, _: JClass, session: jlong, button: jint, down: jboolean, x: jint, y: jint) {
    mobile::send(session as u64, Event::Button { button: button as u8, down: down != JNI_FALSE, x, y }.encode());
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_abrahmch1_kast_Native_wheel(_: JNIEnv, _: JClass, session: jlong, dx: jint, dy: jint) {
    mobile::send(session as u64, Event::Wheel { dx, dy }.encode());
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_abrahmch1_kast_Native_key(mut env: JNIEnv, _: JClass, session: jlong, code: JString, down: jboolean) {
    let code = string(&mut env, &code);
    mobile::send(session as u64, Event::Key { down: down != JNI_FALSE, code }.encode());
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_abrahmch1_kast_Native_text(mut env: JNIEnv, _: JClass, session: jlong, text: JString) {
    let text = string(&mut env, &text);
    mobile::text(session as u64, &text);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_abrahmch1_kast_Native_clipText(mut env: JNIEnv, _: JClass, session: jlong, text: JString) {
    let text = string(&mut env, &text);
    mobile::clip(session as u64, 1, text.as_bytes());
}

/// Fotograma del codificador de esta pantalla: `buf` es el búfer (directo) de salida de MediaCodec.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_abrahmch1_kast_Native_hostFrame(env: JNIEnv, _: JClass, buf: JByteBuffer, offset: jint, size: jint, flags: jint) {
    let Some(h) = HOST.get() else { return };
    let (Ok(ptr), Ok(cap)) = (env.get_direct_buffer_address(&buf), env.get_direct_buffer_capacity(&buf)) else { return };
    let (offset, size) = (offset.max(0) as usize, size.max(0) as usize);
    if ptr.is_null() || offset + size > cap { return; }
    h.frame(unsafe { std::slice::from_raw_parts(ptr.add(offset), size) }, flags);
}
