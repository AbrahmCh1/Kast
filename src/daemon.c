/* kastd.exe — icono en la bandeja, atajos globales y ventana de configuración.
 * Todo comando (atajo, menú o UI) se ejecuta en un único hilo de trabajo para no bloquear la bandeja. */
#include "core.h"
#include "webui.h"
#include <shellapi.h>
#include <windowsx.h>

enum { ID_OPEN = 1, ID_EXTEND, ID_CLONE, ID_OFF, ID_EXIT, ID_FAV = 100 };
enum { T_EXEC, T_HOTKEY, T_STATE };

typedef struct Task { struct Task *next; int kind; char *text; } Task;

static HWND g_hwnd;
static UINT wm_taskbar;
static NOTIFYICONDATAW nid;
static Sb g_log; /* protegido por cfg_cs */
static int hk_count, hk_paused;
static Task *q_head, *q_tail;
static CRITICAL_SECTION q_cs;
static HANDLE q_ev;

static void trim_ws(void) { SetProcessWorkingSetSize(GetCurrentProcess(), (SIZE_T)-1, (SIZE_T)-1); }

/* ---------- cola de trabajo ---------- */

static void q_push(int kind, const char *text)
{
    Task *t = calloc(1, sizeof *t);
    t->kind = kind;
    t->text = text ? _strdup(text) : NULL;
    EnterCriticalSection(&q_cs);
    if (q_tail) q_tail->next = t;
    else q_head = t;
    q_tail = t;
    LeaveCriticalSection(&q_cs);
    SetEvent(q_ev);
}

static Task *q_pop(void)
{
    EnterCriticalSection(&q_cs);
    Task *t = q_head;
    if (t) {
        q_head = t->next;
        if (!q_head) q_tail = NULL;
    }
    LeaveCriticalSection(&q_cs);
    return t;
}

/* ---------- Kast Link: agente en segundo plano (stdin/stdout, una línea JSON por mensaje) ---------- */

#define TIMER_LINK 1
static HANDLE link_in, link_proc;
static int link_missing, link_restarts, quitting;

static DWORD WINAPI link_reader(LPVOID arg)
{
    HANDLE out = arg;
    char buf[4096];
    Sb line = { 0 };
    DWORD n;
    while (ReadFile(out, buf, sizeof buf, &n, NULL) && n) {
        for (DWORD i = 0; i < n; i++) {
            if (buf[i] == '\n') {
                if (line.n) {
                    char *l = _strdup(line.p);
                    if (!PostMessageA(g_hwnd, WM_KAST_LINK, 0, (LPARAM)l)) free(l);
                }
                line.n = 0;
                line.p[0] = 0;
            } else if (buf[i] != '\r') {
                sb_add(&line, &buf[i], 1);
            }
        }
    }
    CloseHandle(out);
    free(line.p);
    PostMessageA(g_hwnd, WM_KAST_LINK, 0, 0);
    return 0;
}

static void link_start(void)
{
    char exe[MAX_PATH], cmd[MAX_PATH + 16];
    snprintf(exe, sizeof exe, "%s\\kast-link.exe", exedir);
    if (GetFileAttributesA(exe) == INVALID_FILE_ATTRIBUTES) { link_missing = 1; return; }
    SECURITY_ATTRIBUTES sa = { sizeof sa, NULL, TRUE };
    HANDLE in_r, in_w, out_r, out_w;
    if (!CreatePipe(&in_r, &in_w, &sa, 0)) return;
    if (!CreatePipe(&out_r, &out_w, &sa, 0)) { CloseHandle(in_r); CloseHandle(in_w); return; }
    SetHandleInformation(in_w, HANDLE_FLAG_INHERIT, 0);
    SetHandleInformation(out_r, HANDLE_FLAG_INHERIT, 0);
    HANDLE nul = CreateFileA("NUL", GENERIC_WRITE, FILE_SHARE_WRITE, &sa, OPEN_EXISTING, 0, NULL);
    STARTUPINFOA si = { .cb = sizeof si, .dwFlags = STARTF_USESTDHANDLES, .hStdInput = in_r, .hStdOutput = out_w, .hStdError = nul };
    PROCESS_INFORMATION pi;
    snprintf(cmd, sizeof cmd, "\"%s\" agent", exe);
    BOOL ok = CreateProcessA(exe, cmd, NULL, NULL, TRUE, CREATE_NO_WINDOW, NULL, exedir, &si, &pi);
    CloseHandle(in_r);
    CloseHandle(out_w);
    CloseHandle(nul);
    if (!ok) { CloseHandle(in_w); CloseHandle(out_r); link_missing = 1; return; }
    CloseHandle(pi.hThread);
    link_proc = pi.hProcess;
    link_in = in_w;
    link_missing = 0;
    CloseHandle(CreateThread(NULL, 64 * 1024, link_reader, out_r, STACK_SIZE_PARAM_IS_A_RESERVATION, NULL));
}

static void link_send(const char *json)
{
    DWORD w;
    if (!link_in) return;
    /* Una sola escritura por línea: también se envía desde el hilo de trabajo (disposiciones). */
    size_t n = strlen(json);
    char *line = malloc(n + 1);
    memcpy(line, json, n);
    line[n] = '\n';
    WriteFile(link_in, line, (DWORD)n + 1, &w, NULL);
    free(line);
}

/* Disposición guardada o aplicada: el agente guarda o restaura sus sesiones (layouts\<nombre>.link). */
static void link_layout(const char *op, const char *path)
{
    Sb s = { 0 };
    sb_printf(&s, "{\"cmd\":\"layout_%s\",\"path\":", op);
    sb_json(&s, path);
    sb_add(&s, "}", 1);
    link_send(s.p);
    free(s.p);
}

/* Cerrar su stdin hace que el agente termine. */
static void link_stop(void)
{
    if (link_in) { CloseHandle(link_in); link_in = NULL; }
    if (link_proc) {
        if (WaitForSingleObject(link_proc, 2000) == WAIT_TIMEOUT) TerminateProcess(link_proc, 0);
        CloseHandle(link_proc);
        link_proc = NULL;
    }
}

static void post_ui(char *json)
{
    if (!PostMessageA(g_hwnd, WM_KAST_RESULT, 0, (LPARAM)json)) free(json);
}

static DWORD WINAPI worker(LPVOID arg)
{
    (void)arg;
    for (;;) {
        WaitForSingleObject(q_ev, INFINITE);
        Task *t;
        while ((t = q_pop())) {
            if (t->kind == T_STATE) {
                EnterCriticalSection(&cfg_cs);
                char *log = _strdup(g_log.p ? g_log.p : "");
                LeaveCriticalSection(&cfg_cs);
                post_ui(state_json(log));
                free(log);
            } else if (t->kind == T_EXEC) {
                out_capture();
                int rc = exec_line(t->text);
                char *out = out_take();
                Sb s = { 0 };
                sb_printf(&s, "{\"t\":\"done\",\"rc\":%d,\"out\":", rc);
                sb_json(&s, out);
                sb_add(&s, "}", 1);
                free(out);
                post_ui(s.p);
            } else {
                exec_line(t->text);
                post_ui(_strdup("{\"t\":\"ext\"}"));
            }
            free(t->text);
            free(t);
        }
        free_phys();
        trim_ws();
    }
    return 0;
}

/* ---------- atajos y configuración ---------- */

static int hotkeys_set(int on, Sb *log)
{
    for (int i = 0; i < hk_count; i++) UnregisterHotKey(g_hwnd, i + 1);
    hk_count = 0;
    if (!on) return 0;
    int ok = 0;
    EnterCriticalSection(&cfg_cs);
    for (int i = 0; i < nhk; i++) {
        if (RegisterHotKey(g_hwnd, i + 1, hks[i].mods, hks[i].vk)) ok++;
        else if (log) sb_printf(log, "'%s' ya está en uso por otro programa\n", hks[i].combo);
    }
    hk_count = nhk;
    LeaveCriticalSection(&cfg_cs);
    return ok;
}

static void reload_config(void)
{
    Sb log = { 0 };
    sb_add(&log, "", 0);
    load_config(&log);
    int ok = hotkeys_set(!hk_paused, &log);
    sb_printf(&log, "%d atajos activos, %d favoritos\n", ok, nfav);
    char path[MAX_PATH];
    snprintf(path, sizeof path, "%s\\kast.log", exedir);
    FILE *f = fopen(path, "w");
    if (f) fputs(log.p, f), fclose(f);
    EnterCriticalSection(&cfg_cs);
    free(g_log.p);
    g_log = log;
    LeaveCriticalSection(&cfg_cs);
}

/* ---------- mensajes de la UI ---------- */

static void reply_log(const char *type)
{
    Sb s = { 0 };
    sb_printf(&s, "{\"t\":\"%s\",\"autostart\":%s,\"log\":", type, autostart_get() ? "true" : "false");
    EnterCriticalSection(&cfg_cs);
    sb_json(&s, g_log.p);
    LeaveCriticalSection(&cfg_cs);
    sb_add(&s, "}", 1);
    ui_post_json(s.p);
    free(s.p);
}

void on_ui_message(const wchar_t *w)
{
    int n = WideCharToMultiByte(CP_UTF8, 0, w, -1, NULL, 0, NULL, NULL);
    char *s = malloc(n);
    WideCharToMultiByte(CP_UTF8, 0, w, -1, s, n, NULL, NULL);
    char *arg = strchr(s, '\n');
    if (arg) *arg++ = 0;
    else arg = s + strlen(s);

    if (!strcmp(s, "state")) q_push(T_STATE, NULL);
    else if (!strcmp(s, "exec")) q_push(T_EXEC, arg);
    else if (!strcmp(s, "save")) {
        char path[MAX_PATH];
        ini_path(path);
        FILE *f = fopen(path, "wb");
        if (f) fwrite(arg, 1, strlen(arg), f), fclose(f);
        reload_config();
        reply_log("saved");
    } else if (!strcmp(s, "capture")) {
        hk_paused = *arg == '1';
        hotkeys_set(!hk_paused, NULL);
    } else if (!strcmp(s, "autostart")) {
        autostart_set(*arg == '1');
        reply_log("autostart");
    } else if (!strcmp(s, "vanish")) {
        /* Opción de Windows "Ocultar el puntero mientras se escribe" */
        SystemParametersInfoW(SPI_SETMOUSEVANISH, 0, (PVOID)(UINT_PTR)(*arg == '1'), SPIF_UPDATEINIFILE | SPIF_SENDCHANGE);
        q_push(T_STATE, NULL);
    } else if (!strcmp(s, "link")) {
        if (link_in) link_send(arg);
        else if (link_missing) ui_post_json("{\"t\":\"link\",\"ev\":\"missing\"}");
        else ui_post_json("{\"t\":\"link\",\"ev\":\"toast\",\"msg\":\"Kast Link se está reiniciando\",\"err\":true}");
    } else if (!strcmp(s, "folder")) {
        ShellExecuteA(NULL, "open", exedir, NULL, NULL, SW_SHOWNORMAL);
    }
    free(s);
}

void on_ui_closed(void)
{
    if (hk_paused) {
        hk_paused = 0;
        hotkeys_set(1, NULL);
    }
    trim_ws();
}

/* ---------- bandeja ---------- */

static void tray_add(void)
{
    UINT dpi = GetDpiForSystem();
    int cx = GetSystemMetricsForDpi(SM_CXSMICON, dpi);
    memset(&nid, 0, sizeof nid);
    nid.cbSize = sizeof nid;
    nid.hWnd = g_hwnd;
    nid.uID = 1;
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
    nid.uCallbackMessage = WM_KAST_TRAY;
    nid.hIcon = LoadImageW(GetModuleHandleW(NULL), MAKEINTRESOURCEW(1), IMAGE_ICON, cx, cx, 0);
    wcscpy(nid.szTip, L"Kast — pantallas");
    Shell_NotifyIconW(NIM_ADD, &nid);
    nid.uVersion = NOTIFYICON_VERSION_4;
    Shell_NotifyIconW(NIM_SETVERSION, &nid);
}

static void menu_add(HMENU m, UINT flags, UINT id, const char *utf8)
{
    wchar_t w[128];
    MultiByteToWideChar(CP_UTF8, 0, utf8, -1, w, 128);
    AppendMenuW(m, flags, id, w);
}

static void tray_menu(int x, int y)
{
    HMENU m = CreatePopupMenu();
    AppendMenuW(m, MF_STRING, ID_OPEN, L"Abrir Kast");
    SetMenuDefaultItem(m, ID_OPEN, FALSE);
    AppendMenuW(m, MF_SEPARATOR, 0, NULL);
    EnterCriticalSection(&cfg_cs);
    int nf = nfav;
    for (int i = 0; i < nfav && i < 30; i++) menu_add(m, MF_STRING, ID_FAV + i, favs[i].name);
    LeaveCriticalSection(&cfg_cs);
    if (nf) AppendMenuW(m, MF_SEPARATOR, 0, NULL);
    AppendMenuW(m, MF_STRING, ID_EXTEND, L"Extender pantallas");
    AppendMenuW(m, MF_STRING, ID_CLONE, L"Duplicar pantallas");
    AppendMenuW(m, MF_STRING, ID_OFF, L"Apagar pantallas");
    AppendMenuW(m, MF_SEPARATOR, 0, NULL);
    AppendMenuW(m, MF_STRING, ID_EXIT, L"Salir");

    SetForegroundWindow(g_hwnd);
    UINT align = GetSystemMetrics(SM_MENUDROPALIGNMENT) ? TPM_RIGHTALIGN : TPM_LEFTALIGN;
    int cmd = TrackPopupMenuEx(m, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | align, x, y, g_hwnd, NULL);
    PostMessageW(g_hwnd, WM_NULL, 0, 0);
    DestroyMenu(m);

    switch (cmd) {
    case ID_OPEN: ui_open(); break;
    case ID_EXTEND: q_push(T_HOTKEY, "topo extend"); break;
    case ID_CLONE: q_push(T_HOTKEY, "topo clone"); break;
    case ID_OFF: q_push(T_HOTKEY, "off"); break;
    case ID_EXIT: PostMessageW(g_hwnd, WM_CLOSE, 0, 0); break;
    default:
        if (cmd >= ID_FAV) {
            char line[300] = "";
            EnterCriticalSection(&cfg_cs);
            if (cmd - ID_FAV < nfav) snprintf(line, sizeof line, "fav %s", favs[cmd - ID_FAV].name);
            LeaveCriticalSection(&cfg_cs);
            if (*line) q_push(T_HOTKEY, line);
        }
    }
}

/* Menús contextuales oscuros (uxtheme, ordinales no documentados pero estables desde 1903). */
static void allow_dark_menus(void)
{
    HMODULE ux = LoadLibraryExW(L"uxtheme.dll", NULL, LOAD_LIBRARY_SEARCH_SYSTEM32);
    if (!ux) return;
    typedef int(WINAPI * SetPreferredAppMode_t)(int);
    typedef void(WINAPI * FlushMenuThemes_t)(void);
    SetPreferredAppMode_t set = (SetPreferredAppMode_t)(void *)GetProcAddress(ux, MAKEINTRESOURCEA(135));
    FlushMenuThemes_t flush = (FlushMenuThemes_t)(void *)GetProcAddress(ux, MAKEINTRESOURCEA(136));
    if (set) set(1);
    if (flush) flush();
}

static LRESULT CALLBACK wndproc(HWND h, UINT msg, WPARAM wp, LPARAM lp)
{
    switch (msg) {
    case WM_KAST_TRAY:
        switch (LOWORD(lp)) {
        case NIN_SELECT:
        case NIN_KEYSELECT: ui_open(); break;
        case WM_CONTEXTMENU: tray_menu(GET_X_LPARAM(wp), GET_Y_LPARAM(wp)); break;
        }
        return 0;
    case WM_HOTKEY: {
        char *act = NULL;
        EnterCriticalSection(&cfg_cs);
        if (wp >= 1 && (int)wp <= nhk) act = _strdup(hks[wp - 1].act);
        LeaveCriticalSection(&cfg_cs);
        if (act) q_push(T_HOTKEY, act), free(act);
        return 0;
    }
    case WM_KAST_OPENUI: ui_open(); return 0;
    case WM_COPYDATA: {
        /* kast.exe layout save|load: "save\n<ruta>" */
        const COPYDATASTRUCT *cd = (const COPYDATASTRUCT *)lp;
        if (cd->dwData != KAST_COPY_LINK_LAYOUT || !cd->cbData || ((const char *)cd->lpData)[cd->cbData - 1]) return FALSE;
        char *op = _strdup(cd->lpData), *path = strchr(op, '\n');
        if (path) {
            *path++ = 0;
            if (!strcmp(op, "save") || !strcmp(op, "load")) link_layout(op, path);
        }
        free(op);
        return TRUE;
    }
    case WM_KAST_RESULT:
        ui_post_json((const char *)lp);
        free((void *)lp);
        return 0;
    case WM_KAST_RELOAD: reload_config(); return 0;
    case WM_KAST_LINK:
        if (!lp) {
            /* El agente terminó: se reintenta unas veces (por si falló al arrancar). */
            if (link_in) { CloseHandle(link_in); link_in = NULL; }
            if (link_proc) { CloseHandle(link_proc); link_proc = NULL; }
            if (!quitting && link_restarts++ < 5) SetTimer(h, TIMER_LINK, 3000, NULL);
            return 0;
        }
        /* Alguien pide ver nuestras pantallas: se muestra la ventana para aceptar o rechazar. */
        if (strstr((char *)lp, "\"ev\":\"request\"") || strstr((char *)lp, "\"ev\":\"show\"")) ui_open();
        ui_post_json((const char *)lp);
        free((void *)lp);
        return 0;
    case WM_TIMER:
        if (wp == TIMER_LINK) { KillTimer(h, TIMER_LINK); link_start(); }
        return 0;
    case WM_DISPLAYCHANGE:
        cache_clear();
        if (ui_is_open()) q_push(T_STATE, NULL);
        break;
    case WM_CLOSE:
        quitting = 1;
        ui_close();
        link_stop();
        DestroyWindow(h);
        return 0;
    case WM_DESTROY:
        Shell_NotifyIconW(NIM_DELETE, &nid);
        PostQuitMessage(0);
        return 0;
    default:
        if (msg == wm_taskbar) tray_add();
    }
    return DefWindowProcW(h, msg, wp, lp);
}

int main(int argc, char **argv)
{
    core_init();
    CreateMutexA(NULL, TRUE, "Local\\KastDaemon");
    if (GetLastError() == ERROR_ALREADY_EXISTS) {
        HWND h = find_daemon();
        if (h) PostMessageA(h, WM_KAST_OPENUI, 0, 0);
        return 0;
    }
    SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    allow_dark_menus();
    InitializeCriticalSection(&q_cs);
    q_ev = CreateEventW(NULL, FALSE, FALSE, NULL);

    HINSTANCE hi = GetModuleHandleW(NULL);
    WNDCLASSW wc = { .lpfnWndProc = wndproc, .hInstance = hi, .lpszClassName = L"" WNDCLASS_NAME };
    RegisterClassW(&wc);
    g_hwnd = CreateWindowExW(WS_EX_TOOLWINDOW, wc.lpszClassName, L"kast", WS_POPUP, 0, 0, 0, 0, NULL, NULL, hi, NULL);
    if (!g_hwnd) return 1;
    wm_taskbar = RegisterWindowMessageW(L"TaskbarCreated");

    reload_config();
    tray_add();
    link_layout_hook = link_layout;
    link_start();
    CloseHandle(CreateThread(NULL, 256 * 1024, worker, NULL, STACK_SIZE_PARAM_IS_A_RESERVATION, NULL));
    if (argc > 1 && !strcmp(argv[1], "--ui")) ui_open();
    trim_ws();

    MSG m;
    while (GetMessageW(&m, NULL, 0, 0) > 0) {
        TranslateMessage(&m);
        DispatchMessageW(&m);
    }
    return 0;
}
