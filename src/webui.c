/* Ventana de configuración: WebView2 alojado en C puro (COM con vtables a mano).
 * El navegador sólo existe mientras la ventana está abierta; al cerrarla se liberan sus procesos. */
#define COBJMACROS
#include "core.h"
#include "webui.h"
#include <dwmapi.h>
#include <shellscalingapi.h>
#include "WebView2.h"

#define UI_CLASS L"KastUI"
#define DWMWA_DARK 20
#define DWMWA_BACKDROP 38

typedef HRESULT(STDAPICALLTYPE *CreateEnv_t)(PCWSTR, PCWSTR, ICoreWebView2EnvironmentOptions *,
                                             ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler *);

static HWND ui_hwnd;
static ICoreWebView2Environment *env;
static ICoreWebView2Controller *ctrl;
static ICoreWebView2 *wv;
static EventRegistrationToken msg_tok;
static HMODULE wv_dll;
static int ui_dark, ui_mica;

/* ---------- manejadores COM estáticos (sin conteo de referencias) ---------- */

static HRESULT STDMETHODCALLTYPE h_qi(void *self, REFIID riid, void **ppv) { (void)riid; *ppv = self; return S_OK; }
static ULONG STDMETHODCALLTYPE h_ref(void *self) { (void)self; return 1; }
#define QI(T)  (HRESULT(STDMETHODCALLTYPE *)(T *, REFIID, void **))(void *)h_qi
#define REF(T) (ULONG(STDMETHODCALLTYPE *)(T *))(void *)h_ref

static HRESULT STDMETHODCALLTYPE env_done(ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler *, HRESULT, ICoreWebView2Environment *);
static HRESULT STDMETHODCALLTYPE ctrl_done(ICoreWebView2CreateCoreWebView2ControllerCompletedHandler *, HRESULT, ICoreWebView2Controller *);
static HRESULT STDMETHODCALLTYPE msg_recv(ICoreWebView2WebMessageReceivedEventHandler *, ICoreWebView2 *, ICoreWebView2WebMessageReceivedEventArgs *);

static ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandlerVtbl env_vt = {
    QI(ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler),
    REF(ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler),
    REF(ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler), env_done,
};
static ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler env_h = { &env_vt };

static ICoreWebView2CreateCoreWebView2ControllerCompletedHandlerVtbl ctrl_vt = {
    QI(ICoreWebView2CreateCoreWebView2ControllerCompletedHandler),
    REF(ICoreWebView2CreateCoreWebView2ControllerCompletedHandler),
    REF(ICoreWebView2CreateCoreWebView2ControllerCompletedHandler), ctrl_done,
};
static ICoreWebView2CreateCoreWebView2ControllerCompletedHandler ctrl_h = { &ctrl_vt };

static ICoreWebView2WebMessageReceivedEventHandlerVtbl msg_vt = {
    QI(ICoreWebView2WebMessageReceivedEventHandler),
    REF(ICoreWebView2WebMessageReceivedEventHandler),
    REF(ICoreWebView2WebMessageReceivedEventHandler), msg_recv,
};
static ICoreWebView2WebMessageReceivedEventHandler msg_h = { &msg_vt };

/* ---------- ayuda ---------- */

static void ui_fail(const wchar_t *what, HRESULT hr)
{
    wchar_t m[256];
    swprintf(m, 256, L"%ls (0x%08lX).\n\nKast necesita el runtime de Microsoft Edge WebView2.", what, (unsigned long)hr);
    MessageBoxW(ui_hwnd, m, L"Kast", MB_ICONERROR);
    ui_close();
}

static wchar_t *load_html(void)
{
    HRSRC r = FindResourceW(NULL, L"UI", MAKEINTRESOURCEW(10)); /* RT_RCDATA */
    if (!r) return NULL;
    DWORD n = SizeofResource(NULL, r);
    const char *p = LockResource(LoadResource(NULL, r));
    int wn = MultiByteToWideChar(CP_UTF8, 0, p, (int)n, NULL, 0);
    wchar_t *w = malloc((wn + 1) * sizeof *w);
    MultiByteToWideChar(CP_UTF8, 0, p, (int)n, w, wn);
    w[wn] = 0;
    return w;
}

static void resize(void)
{
    if (!ctrl) return;
    RECT r;
    GetClientRect(ui_hwnd, &r);
    ICoreWebView2Controller_put_Bounds(ctrl, r);
}

static int system_dark(void)
{
    DWORD v = 1, sz = sizeof v;
    RegGetValueW(HKEY_CURRENT_USER, L"Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
                 L"AppsUseLightTheme", RRF_RT_REG_DWORD, NULL, &v, &sz);
    return v == 0;
}

/* ---------- callbacks WebView2 ---------- */

static HRESULT STDMETHODCALLTYPE env_done(ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler *self,
                                          HRESULT hr, ICoreWebView2Environment *e)
{
    (void)self;
    if (!ui_hwnd) return S_OK;
    if (FAILED(hr) || !e) { ui_fail(L"No se pudo crear el entorno WebView2", hr); return S_OK; }
    env = e;
    ICoreWebView2Environment_AddRef(e);
    hr = ICoreWebView2Environment_CreateCoreWebView2Controller(e, ui_hwnd, &ctrl_h);
    if (FAILED(hr)) ui_fail(L"No se pudo crear el control WebView2", hr);
    return S_OK;
}

static HRESULT STDMETHODCALLTYPE ctrl_done(ICoreWebView2CreateCoreWebView2ControllerCompletedHandler *self,
                                           HRESULT hr, ICoreWebView2Controller *c)
{
    (void)self;
    if (!ui_hwnd) {
        if (c) ICoreWebView2Controller_Close(c);
        return S_OK;
    }
    if (FAILED(hr) || !c) { ui_fail(L"No se pudo crear el control WebView2", hr); return S_OK; }
    ctrl = c;
    ICoreWebView2Controller_AddRef(c);
    ICoreWebView2Controller_get_CoreWebView2(c, &wv);

    ICoreWebView2Settings *st;
    if (SUCCEEDED(ICoreWebView2_get_Settings(wv, &st))) {
        ICoreWebView2Settings_put_AreDevToolsEnabled(st, FALSE);
        ICoreWebView2Settings_put_AreDefaultContextMenusEnabled(st, FALSE);
        ICoreWebView2Settings_put_IsStatusBarEnabled(st, FALSE);
        ICoreWebView2Settings_put_IsZoomControlEnabled(st, FALSE);
        ICoreWebView2Settings3 *s3;
        if (SUCCEEDED(ICoreWebView2Settings_QueryInterface(st, &IID_ICoreWebView2Settings3, (void **)&s3))) {
            ICoreWebView2Settings3_put_AreBrowserAcceleratorKeysEnabled(s3, FALSE);
            ICoreWebView2Settings3_Release(s3);
        }
        ICoreWebView2Settings_Release(st);
    }
    ICoreWebView2Controller2 *c2;
    if (SUCCEEDED(ICoreWebView2Controller_QueryInterface(c, &IID_ICoreWebView2Controller2, (void **)&c2))) {
        COREWEBVIEW2_COLOR col = { ui_mica ? 0 : 255, ui_dark ? 0x20 : 0xF3, ui_dark ? 0x20 : 0xF3, ui_dark ? 0x20 : 0xF3 };
        ICoreWebView2Controller2_put_DefaultBackgroundColor(c2, col);
        ICoreWebView2Controller2_Release(c2);
    }
    ICoreWebView2_add_WebMessageReceived(wv, &msg_h, &msg_tok);
    wchar_t ver[32], init[128];
    MultiByteToWideChar(CP_UTF8, 0, KAST_VERSION, -1, ver, 32);
    swprintf(init, 128, L"window.KAST={mica:%d,version:'%ls'};", ui_mica, ver);
    ICoreWebView2_AddScriptToExecuteOnDocumentCreated(wv, init, NULL);
    resize();
    wchar_t *html = load_html();
    if (html) {
        ICoreWebView2_NavigateToString(wv, html);
        free(html);
    }
    ICoreWebView2Controller_MoveFocus(c, COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
    return S_OK;
}

static HRESULT STDMETHODCALLTYPE msg_recv(ICoreWebView2WebMessageReceivedEventHandler *self, ICoreWebView2 *sender,
                                          ICoreWebView2WebMessageReceivedEventArgs *args)
{
    (void)self; (void)sender;
    LPWSTR s = NULL;
    if (SUCCEEDED(ICoreWebView2WebMessageReceivedEventArgs_TryGetWebMessageAsString(args, &s)) && s) {
        on_ui_message(s);
        CoTaskMemFree(s);
    }
    return S_OK;
}

/* ---------- ventana ---------- */

static LRESULT CALLBACK ui_proc(HWND h, UINT msg, WPARAM wp, LPARAM lp)
{
    switch (msg) {
    case WM_SIZE:
        resize();
        return 0;
    case WM_MOVE:
        if (ctrl) ICoreWebView2Controller_NotifyParentWindowPositionChanged(ctrl);
        break;
    case WM_ACTIVATE:
        if (ctrl && LOWORD(wp) != WA_INACTIVE)
            ICoreWebView2Controller_MoveFocus(ctrl, COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
        break;
    case WM_DPICHANGED: {
        RECT *r = (RECT *)lp;
        SetWindowPos(h, NULL, r->left, r->top, r->right - r->left, r->bottom - r->top, SWP_NOZORDER | SWP_NOACTIVATE);
        return 0;
    }
    case WM_GETMINMAXINFO: {
        UINT dpi = GetDpiForWindow(h);
        MINMAXINFO *mm = (MINMAXINFO *)lp;
        mm->ptMinTrackSize.x = MulDiv(760, dpi, 96);
        mm->ptMinTrackSize.y = MulDiv(520, dpi, 96);
        return 0;
    }
    case WM_ERASEBKGND: {
        RECT r;
        GetClientRect(h, &r);
        HBRUSH b = CreateSolidBrush(ui_mica ? RGB(0, 0, 0) : ui_dark ? RGB(0x20, 0x20, 0x20) : RGB(0xF3, 0xF3, 0xF3));
        FillRect((HDC)wp, &r, b);
        DeleteObject(b);
        return 1;
    }
    case WM_SETTINGCHANGE:
        if (lp && !lstrcmpW((LPCWSTR)lp, L"ImmersiveColorSet")) {
            BOOL d = ui_dark = system_dark();
            DwmSetWindowAttribute(h, DWMWA_DARK, &d, sizeof d);
        }
        break;
    case WM_CLOSE:
        ui_close();
        return 0;
    }
    return DefWindowProcW(h, msg, wp, lp);
}

int ui_is_open(void) { return ui_hwnd != NULL; }

void ui_open(void)
{
    if (ui_hwnd) {
        if (IsIconic(ui_hwnd)) ShowWindow(ui_hwnd, SW_RESTORE);
        SetForegroundWindow(ui_hwnd);
        return;
    }
    static int inited;
    HINSTANCE hi = GetModuleHandleW(NULL);
    if (!inited) {
        CoInitializeEx(NULL, COINIT_APARTMENTTHREADED);
        WNDCLASSEXW wc = { .cbSize = sizeof wc };
        wc.style = CS_HREDRAW | CS_VREDRAW;
        wc.lpfnWndProc = ui_proc;
        wc.hInstance = hi;
        wc.hIcon = LoadImageW(hi, MAKEINTRESOURCEW(1), IMAGE_ICON, 0, 0, LR_DEFAULTSIZE);
        wc.hIconSm = LoadImageW(hi, MAKEINTRESOURCEW(1), IMAGE_ICON, GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON), 0);
        wc.hCursor = LoadCursorW(NULL, (LPCWSTR)IDC_ARROW);
        wc.lpszClassName = UI_CLASS;
        RegisterClassExW(&wc);
        inited = 1;
    }
    if (!wv_dll) {
        wchar_t p[MAX_PATH];
        GetModuleFileNameW(NULL, p, MAX_PATH);
        wchar_t *sl = wcsrchr(p, L'\\');
        if (sl) wcscpy(sl + 1, L"WebView2Loader.dll");
        wv_dll = LoadLibraryW(p);
        if (!wv_dll) { MessageBoxW(NULL, L"Falta WebView2Loader.dll junto a kastd.exe.", L"Kast", MB_ICONERROR); return; }
    }
    CreateEnv_t create = (CreateEnv_t)(void *)GetProcAddress(wv_dll, "CreateCoreWebView2EnvironmentWithOptions");
    if (!create) return;

    POINT pt;
    GetCursorPos(&pt);
    HMONITOR hm = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
    MONITORINFO mi = { .cbSize = sizeof mi };
    GetMonitorInfoW(hm, &mi);
    UINT dpi = 96, dpy;
    GetDpiForMonitor(hm, MDT_EFFECTIVE_DPI, &dpi, &dpy);
    RECT wa = mi.rcWork;
    int w = MulDiv(1100, dpi, 96), h = MulDiv(760, dpi, 96);
    if (w > wa.right - wa.left) w = wa.right - wa.left;
    if (h > wa.bottom - wa.top) h = wa.bottom - wa.top;
    int x = wa.left + (wa.right - wa.left - w) / 2, y = wa.top + (wa.bottom - wa.top - h) / 2;

    ui_dark = system_dark();
    ui_hwnd = CreateWindowExW(0, UI_CLASS, L"Kast", WS_OVERLAPPEDWINDOW, x, y, w, h, NULL, NULL, hi, NULL);
    if (!ui_hwnd) return;
    BOOL d = ui_dark;
    DwmSetWindowAttribute(ui_hwnd, DWMWA_DARK, &d, sizeof d);
    int backdrop = 2; /* DWMSBT_MAINWINDOW = Mica */
    ui_mica = SUCCEEDED(DwmSetWindowAttribute(ui_hwnd, DWMWA_BACKDROP, &backdrop, sizeof backdrop));
    if (ui_mica) {
        MARGINS m = { -1, -1, -1, -1 };
        DwmExtendFrameIntoClientArea(ui_hwnd, &m);
    }
    ShowWindow(ui_hwnd, SW_SHOWNORMAL); /* el primer ShowWindow puede heredar el modo de arranque */
    if (IsIconic(ui_hwnd) || !IsWindowVisible(ui_hwnd)) ShowWindow(ui_hwnd, SW_RESTORE);
    UpdateWindow(ui_hwnd);
    SetForegroundWindow(ui_hwnd);

    wchar_t data[MAX_PATH];
    ExpandEnvironmentStringsW(L"%LOCALAPPDATA%\\Kast\\WebView2", data, MAX_PATH);
    HRESULT hr = create(NULL, data, NULL, &env_h);
    if (FAILED(hr)) ui_fail(L"No se pudo iniciar WebView2", hr);
}

void ui_close(void)
{
    if (!ui_hwnd) return;
    HWND h = ui_hwnd;
    ui_hwnd = NULL;
    if (wv) {
        ICoreWebView2_remove_WebMessageReceived(wv, msg_tok);
        ICoreWebView2_Release(wv);
        wv = NULL;
    }
    if (ctrl) {
        ICoreWebView2Controller_Close(ctrl);
        ICoreWebView2Controller_Release(ctrl);
        ctrl = NULL;
    }
    if (env) {
        ICoreWebView2Environment_Release(env);
        env = NULL;
    }
    DestroyWindow(h);
    on_ui_closed();
}

void ui_post_json(const char *json)
{
    if (!wv || !json) return;
    int n = MultiByteToWideChar(CP_UTF8, 0, json, -1, NULL, 0);
    wchar_t *w = malloc(n * sizeof *w);
    MultiByteToWideChar(CP_UTF8, 0, json, -1, w, n);
    ICoreWebView2_PostWebMessageAsJson(wv, w);
    free(w);
}
