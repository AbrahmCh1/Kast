/* kast — control de monitores (Win32 + DDC/CI): monitores, comandos y configuración. */
#include "core.h"
#include <physicalmonitorenumerationapi.h>
#include <lowlevelmonitorconfigurationapi.h>

#define MAXMON   16
#define CACHE_MS 4000   /* el monitor puede cambiar por su menú: la caché sólo agiliza ráfagas de atajos */
#define DDC_GAP_MS 60   /* DDC/CI exige ~50 ms entre órdenes al mismo monitor o las ignora */

typedef struct {
    HMONITOR h;
    char dev[CCHDEVICENAME];
    char name[64];
    RECT rc;
    int primary;
    PHYSICAL_MONITOR *pm;
    DWORD npm;
    int pm_tried;
} Mon;

enum { J_GET, J_SET, J_REL };
/* Calibración: curva de puntos (nivel de Kast en %, valor real en % de la escala del monitor), por tramos rectos. */
#define MAXPTS 8
#define MAXCAL 32
typedef struct { int n; unsigned char k[MAXPTS], r[MAXPTS]; } Curve;
typedef struct { int idx; BYTE code; int mode, pct; Curve cv; long v; DWORD cur, max; int ok; } Job;
typedef struct { char name[64]; Curve b, c; } Cal;
typedef struct { char dev[CCHDEVICENAME]; DWORD k; BYTE code; DWORD cur, max, tick; } Cache;

char exedir[MAX_PATH];
Fav favs[MAXFAV];
int nfav;
Hotkey hks[MAXHK];
int nhk;
CRITICAL_SECTION cfg_cs;

static Mon mon[MAXMON];
static int nmon;
static Job jobs[MAXMON];
static Cache cache[64];
static int ncache;
static CRITICAL_SECTION cache_cs;
static int cfg_loaded;
static Cal cals[MAXCAL];
static int ncal;
static const Curve ident = { 2, { 0, 100 }, { 0, 100 } };
static struct { char dev[CCHDEVICENAME]; DWORD k, tick; } gaps[32];
static int ngap;
static int fav_depth;
static Sb g_out;
static int g_capture;

/* ---------- utilidades ---------- */

void sb_add(Sb *s, const char *p, size_t n)
{
    if (s->n + n + 1 > s->cap) {
        s->cap = (s->n + n + 1) * 2;
        s->p = realloc(s->p, s->cap);
    }
    memcpy(s->p + s->n, p, n);
    s->n += n;
    s->p[s->n] = 0;
}

void sb_printf(Sb *s, const char *fmt, ...)
{
    char tmp[1024];
    va_list ap;
    va_start(ap, fmt);
    int n = vsnprintf(tmp, sizeof tmp, fmt, ap);
    va_end(ap);
    if (n > 0) sb_add(s, tmp, n < (int)sizeof tmp ? (size_t)n : sizeof tmp - 1);
}

void sb_json(Sb *s, const char *str)
{
    sb_add(s, "\"", 1);
    for (const unsigned char *p = (const unsigned char *)(str ? str : ""); *p; p++) {
        if (*p == '"' || *p == '\\') { char e[2] = { '\\', (char)*p }; sb_add(s, e, 2); }
        else if (*p == '\n') sb_add(s, "\\n", 2);
        else if (*p == '\r') sb_add(s, "\\r", 2);
        else if (*p == '\t') sb_add(s, "\\t", 2);
        else if (*p < 0x20) sb_printf(s, "\\u%04x", *p);
        else sb_add(s, (const char *)p, 1);
    }
    sb_add(s, "\"", 1);
}

int say(const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    int n;
    if (g_capture) {
        char tmp[4096];
        n = vsnprintf(tmp, sizeof tmp, fmt, ap);
        if (n > 0) sb_add(&g_out, tmp, n < (int)sizeof tmp ? (size_t)n : sizeof tmp - 1);
    } else {
        n = vprintf(fmt, ap);
    }
    va_end(ap);
    return n;
}

static int sayl(const char *s) { return say("%s\n", s); }

void out_capture(void)
{
    g_capture = 1;
    g_out.n = 0;
    if (g_out.p) g_out.p[0] = 0;
}

char *out_take(void)
{
    g_capture = 0;
    char *r = _strdup(g_out.p ? g_out.p : "");
    g_out.n = 0;
    if (g_out.p) g_out.p[0] = 0;
    return r;
}

void core_init(void)
{
    InitializeCriticalSection(&cache_cs);
    InitializeCriticalSection(&cfg_cs);
    GetModuleFileNameA(NULL, exedir, sizeof exedir);
    char *sl = strrchr(exedir, '\\');
    if (sl) *sl = 0;
}

/* ---------- enumeración ---------- */

static BOOL CALLBACK enum_cb(HMONITOR h, HDC dc, LPRECT r, LPARAM lp)
{
    (void)dc; (void)r; (void)lp;
    if (nmon >= MAXMON) return FALSE;
    MONITORINFOEXA mi = { .cbSize = sizeof mi };
    if (!GetMonitorInfoA(h, (MONITORINFO *)&mi)) return TRUE;
    Mon *m = &mon[nmon++];
    memset(m, 0, sizeof *m);
    m->h = h;
    m->rc = mi.rcMonitor;
    m->primary = !!(mi.dwFlags & MONITORINFOF_PRIMARY);
    strcpy(m->dev, mi.szDevice);
    strcpy(m->name, mi.szDevice + 4);
    return TRUE;
}

static int moncmp(const void *a, const void *b)
{
    const Mon *x = a, *y = b;
    return x->rc.left != y->rc.left ? x->rc.left - y->rc.left : x->rc.top - y->rc.top;
}

void free_phys(void)
{
    for (int i = 0; i < nmon; i++)
        if (mon[i].pm) {
            DestroyPhysicalMonitors(mon[i].npm, mon[i].pm);
            free(mon[i].pm);
            mon[i].pm = NULL;
        }
}

static void friendly_names(void)
{
    UINT32 np = 0, nm = 0;
    if (GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &np, &nm)) return;
    DISPLAYCONFIG_PATH_INFO *p = calloc(np, sizeof *p);
    DISPLAYCONFIG_MODE_INFO *mo = calloc(nm, sizeof *mo);
    if (!QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &np, p, &nm, mo, NULL)) {
        for (UINT32 i = 0; i < np; i++) {
            DISPLAYCONFIG_SOURCE_DEVICE_NAME s = { 0 };
            s.header.type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            s.header.size = sizeof s;
            s.header.adapterId = p[i].sourceInfo.adapterId;
            s.header.id = p[i].sourceInfo.id;
            DISPLAYCONFIG_TARGET_DEVICE_NAME t = { 0 };
            t.header.type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
            t.header.size = sizeof t;
            t.header.adapterId = p[i].targetInfo.adapterId;
            t.header.id = p[i].targetInfo.id;
            if (DisplayConfigGetDeviceInfo(&s.header) || DisplayConfigGetDeviceInfo(&t.header)) continue;
            if (!t.monitorFriendlyDeviceName[0]) continue;
            char gdi[CCHDEVICENAME];
            WideCharToMultiByte(CP_ACP, 0, s.viewGdiDeviceName, -1, gdi, sizeof gdi, NULL, NULL);
            for (int j = 0; j < nmon; j++)
                if (!_stricmp(mon[j].dev, gdi))
                    WideCharToMultiByte(CP_UTF8, 0, t.monitorFriendlyDeviceName, -1,
                                        mon[j].name, sizeof mon[j].name, NULL, NULL);
        }
    }
    free(p);
    free(mo);
}

static void enum_mons(void)
{
    free_phys();
    nmon = 0;
    EnumDisplayMonitors(NULL, NULL, enum_cb, 0);
    qsort(mon, nmon, sizeof *mon, moncmp);
    friendly_names();
}

static int phys(Mon *m)
{
    if (!m->pm_tried) {
        m->pm_tried = 1;
        DWORD n = 0;
        if (GetNumberOfPhysicalMonitorsFromHMONITOR(m->h, &n) && n) {
            m->pm = calloc(n, sizeof *m->pm);
            if (GetPhysicalMonitorsFromHMONITOR(m->h, n, m->pm)) m->npm = n;
            else { free(m->pm); m->pm = NULL; }
        }
    }
    return m->npm;
}

/* "all" | "*" | "p" | "c" | "1" | "1,3" -> máscara de bits (0 = inválido) */
static unsigned targets(const char *s)
{
    if (!s || !nmon) return 0;
    if (!_stricmp(s, "all") || !strcmp(s, "*")) return (1u << nmon) - 1;
    if (!_stricmp(s, "p")) {
        for (int i = 0; i < nmon; i++) if (mon[i].primary) return 1u << i;
        return 0;
    }
    if (!_stricmp(s, "c")) {
        POINT pt;
        GetCursorPos(&pt);
        HMONITOR h = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        for (int i = 0; i < nmon; i++) if (mon[i].h == h) return 1u << i;
        return 0;
    }
    unsigned mask = 0;
    const char *p = s;
    while (*p) {
        char *e;
        long n = strtol(p, &e, 10);
        if (e == p || n < 1 || n > nmon) return 0;
        mask |= 1u << (n - 1);
        p = e;
        if (*p == ',') p++;
        else if (*p) return 0;
    }
    return mask;
}

static int one_target(const char *s)
{
    unsigned m = targets(s);
    if (!m || (m & (m - 1))) return -1;
    int i = 0;
    while (!(m & 1)) { m >>= 1; i++; }
    return i;
}

/* ---------- DDC/CI ---------- */

static Cache *cache_find(const char *dev, DWORD k, BYTE code, int create)
{
    for (int i = 0; i < ncache; i++)
        if (cache[i].k == k && cache[i].code == code && !strcmp(cache[i].dev, dev)) return &cache[i];
    if (!create) return NULL;
    Cache *c = &cache[ncache < 64 ? ncache++ : 63];
    memset(c, 0, sizeof *c);
    strcpy(c->dev, dev);
    c->k = k;
    c->code = code;
    return c;
}

static int cache_get(const char *dev, DWORD k, BYTE code, DWORD *cur, DWORD *max)
{
    int ok = 0;
    EnterCriticalSection(&cache_cs);
    Cache *c = cache_find(dev, k, code, 0);
    if (c && c->tick && GetTickCount() - c->tick < CACHE_MS) { *cur = c->cur; *max = c->max; ok = 1; }
    else if (c && max) *max = c->max;
    LeaveCriticalSection(&cache_cs);
    return ok;
}

static void cache_put(const char *dev, DWORD k, BYTE code, DWORD cur, DWORD max)
{
    EnterCriticalSection(&cache_cs);
    Cache *c = cache_find(dev, k, code, 1);
    c->cur = cur;
    if (max) c->max = max;
    c->tick = GetTickCount();
    LeaveCriticalSection(&cache_cs);
}

void cache_clear(void)
{
    EnterCriticalSection(&cache_cs);
    ncache = 0;
    LeaveCriticalSection(&cache_cs);
}

/* Espera lo necesario desde la última orden DDC a ese monitor (mark = 0) o registra una orden (mark = 1). */
static void ddc_gap(const char *dev, DWORD k, int mark)
{
    EnterCriticalSection(&cache_cs);
    int i;
    for (i = 0; i < ngap; i++)
        if (gaps[i].k == k && !strcmp(gaps[i].dev, dev)) break;
    if (i == ngap) {
        if (ngap < 32) ngap++;
        else i = 31;
        strcpy(gaps[i].dev, dev);
        gaps[i].k = k;
        gaps[i].tick = 0;
    }
    DWORD last = gaps[i].tick, now = GetTickCount();
    if (mark) gaps[i].tick = now ? now : 1;
    LeaveCriticalSection(&cache_cs);
    if (!mark && last && now - last < DDC_GAP_MS) Sleep(DDC_GAP_MS - (now - last));
}

static int ddc_get(Mon *m, DWORD k, BYTE code, DWORD *cur, DWORD *max)
{
    for (int t = 0; t < 3; t++) {
        ddc_gap(m->dev, k, 0);
        int ok = GetVCPFeatureAndVCPFeatureReply(m->pm[k].hPhysicalMonitor, code, NULL, cur, max);
        ddc_gap(m->dev, k, 1);
        if (ok) return 1;
    }
    return 0;
}

static int ddc_set(Mon *m, DWORD k, BYTE code, DWORD v)
{
    for (int t = 0; t < 3; t++) {
        ddc_gap(m->dev, k, 0);
        int ok = SetVCPFeature(m->pm[k].hPhysicalMonitor, code, v);
        ddc_gap(m->dev, k, 1);
        if (ok) return 1;
    }
    return 0;
}

/* Curva de calibración de un monitor para el brillo (0x10) o el contraste (0x12); sin calibrar, la identidad. */
static void cal_get(const char *name, BYTE code, Curve *cv)
{
    *cv = ident;
    if (code != 0x10 && code != 0x12) return;
    EnterCriticalSection(&cfg_cs);
    for (int i = 0; i < ncal; i++)
        if (!_stricmp(cals[i].name, name)) *cv = code == 0x10 ? cals[i].b : cals[i].c;
    LeaveCriticalSection(&cfg_cs);
}

/* Valor real del monitor (en % de su escala) -> nivel de Kast. */
static long to_user(long raw, const Curve *c)
{
    if (raw <= c->r[0]) return 0;
    for (int i = 0; i + 1 < c->n; i++)
        if (raw <= c->r[i + 1]) {
            long dr = c->r[i + 1] - c->r[i], dk = c->k[i + 1] - c->k[i];
            return c->k[i] + ((raw - c->r[i]) * dk + dr / 2) / dr;
        }
    return 100;
}

/* Nivel de Kast (0-100) -> valor real del monitor (en % de su escala). */
static long to_raw(long user, const Curve *c)
{
    for (int i = 0; i + 1 < c->n; i++)
        if (user <= c->k[i + 1]) {
            long dk = c->k[i + 1] - c->k[i];
            return c->r[i] + ((user - c->k[i]) * (c->r[i + 1] - c->r[i]) + dk / 2) / dk;
        }
    return c->r[c->n - 1];
}

/* "0:10 50:40 100:85" (nivel de Kast:valor real) o, como antes, "10 85" (valores para el 0 % y el 100 %).
 * Los niveles van de 0 a 100 en orden y los valores no bajan; entre el primero y el último, al menos 5 puntos. */
static int parse_curve(const char *s, Curve *c)
{
    c->n = 0;
    if (!strchr(s, ':')) {
        int lo, hi;
        if (sscanf(s, "%d %d", &lo, &hi) != 2 || lo < 0 || hi > 100 || lo > hi) return 0;
        *c = (Curve){ 2, { 0, 100 }, { (unsigned char)lo, (unsigned char)hi } };
    } else {
        for (;;) {
            while (*s == ' ' || *s == '\t') s++;
            if (!*s) break;
            int k, r, used;
            if (c->n == MAXPTS || sscanf(s, "%d:%d%n", &k, &r, &used) != 2 || k < 0 || k > 100 || r < 0 || r > 100) return 0;
            c->k[c->n] = (unsigned char)k;
            c->r[c->n++] = (unsigned char)r;
            s += used;
        }
    }
    if (c->n < 2 || c->k[0] != 0 || c->k[c->n - 1] != 100) return 0;
    for (int i = 1; i < c->n; i++)
        if (c->k[i] <= c->k[i - 1] || c->r[i] < c->r[i - 1]) return 0;
    return c->r[c->n - 1] - c->r[0] >= 5;
}

/* Entrada de calibración de un monitor (se crea sin calibrar si no existe). Con cfg_cs tomado. */
static Cal *cal_slot(const char *name)
{
    for (int i = 0; i < ncal; i++)
        if (!_stricmp(cals[i].name, name)) return &cals[i];
    if (ncal == MAXCAL) return NULL;
    Cal *c = &cals[ncal++];
    snprintf(c->name, sizeof c->name, "%s", name);
    c->b = c->c = ident;
    return c;
}

static DWORD WINAPI job_run(LPVOID arg)
{
    Job *j = arg;
    Mon *m = &mon[j->idx];
    for (DWORD k = 0; k < m->npm; k++) {
        DWORD cur = 0, max = 0;
        int fresh = cache_get(m->dev, k, j->code, &cur, &max);
        int need = j->mode == J_GET || (j->mode == J_REL && !fresh) || (j->pct && !max);
        if (need) {
            if (!ddc_get(m, k, j->code, &cur, &max)) continue;
            cache_put(m->dev, k, j->code, cur, max);
        }
        int pct = j->pct && max;
        long base = pct ? to_user(((long)cur * 100 + max / 2) / max, &j->cv) : (long)cur;
        j->max = pct ? 100 : max;
        if (j->mode == J_GET) { j->cur = base; j->ok = 1; continue; }
        long nv = j->mode == J_REL ? base + j->v : j->v;
        if (nv < 0) nv = 0;
        if (j->max && nv > (long)j->max) nv = j->max;
        DWORD raw = pct ? (DWORD)((to_raw(nv, &j->cv) * (long)max + 50) / 100) : (DWORD)nv;
        if (ddc_set(m, k, j->code, raw)) {
            cache_put(m->dev, k, j->code, raw, max);
            j->cur = nv;
            j->ok = 1;
        }
    }
    return 0;
}

/* Ejecuta la operación en paralelo (un hilo por monitor); resultados en jobs[].
 * pct: 0 = valor crudo, 1 = porcentaje (calibrado si es brillo o contraste), 2 = porcentaje sin calibrar. */
static void run_jobs(unsigned mask, BYTE code, int mode, long v, int pct)
{
    HANDLE th[MAXMON];
    int nt = 0;
    for (int i = 0; i < nmon; i++) {
        jobs[i] = (Job){ .idx = i, .code = code, .mode = mode, .pct = pct != 0, .cv = ident, .v = v };
        if (pct == 1) cal_get(mon[i].name, code, &jobs[i].cv);
        if (!(mask & (1u << i)) || !phys(&mon[i])) continue;
        th[nt] = CreateThread(NULL, 64 * 1024, job_run, &jobs[i], STACK_SIZE_PARAM_IS_A_RESERVATION, NULL);
        if (!th[nt]) job_run(&jobs[i]);
        else nt++;
    }
    if (nt) WaitForMultipleObjects(nt, th, TRUE, INFINITE);
    for (int i = 0; i < nt; i++) CloseHandle(th[i]);
}

static int parse_val(const char *s, int *mode, long *v)
{
    char *e;
    *mode = (s[0] == '+' || s[0] == '-') ? J_REL : J_SET;
    *v = strtol(s, &e, 0);
    return e != s && !*e;
}

static const struct { const char *n; int v; } inputs[] = {
    { "vga1", 0x01 }, { "vga2", 0x02 }, { "dvi1", 0x03 }, { "dvi2", 0x04 },
    { "dp1", 0x0F },  { "dp2", 0x10 },  { "hdmi1", 0x11 }, { "hdmi2", 0x12 },
    { "usbc", 0x1B }, { "usbc2", 0x1C }, { "dp", 0x0F },   { "hdmi", 0x11 },
};

static const char *input_name(DWORD v)
{
    for (size_t i = 0; i < sizeof inputs / sizeof *inputs; i++)
        if (inputs[i].v == (int)(v & 0xFF)) return inputs[i].n;
    return "?";
}

static int bad_target(const char *s)
{
    say("monitor inválido: %s (1-%d, all, p, c)\n", s ? s : "", nmon);
    return 1;
}

static int report(unsigned mask, int mode, BYTE code, int pct)
{
    int rc = 0;
    for (int i = 0; i < nmon; i++) {
        if (!(mask & (1u << i))) continue;
        Job *j = &jobs[i];
        if (!j->ok) { say("%d %s: sin respuesta DDC/CI\n", i + 1, mon[i].name); rc = 1; continue; }
        if (code == 0x60) say("%d %s: %s (0x%02lX)\n", i + 1, mon[i].name, input_name(j->cur), j->cur & 0xFF);
        else if (pct) say("%d %s: %lu%%\n", i + 1, mon[i].name, j->cur);
        else if (mode == J_GET) say("%d %s: %lu/%lu\n", i + 1, mon[i].name, j->cur, j->max);
        else say("%d %s: %lu\n", i + 1, mon[i].name, j->cur);
    }
    return rc;
}

/* b / c / vol / vcp:  [mon] [valor] */
static int cmd_feature(int ac, char **av, BYTE code, int pct)
{
    const char *t = ac > 1 ? av[1] : "all";
    unsigned mask = targets(t);
    if (!mask) return bad_target(t);
    int mode = J_GET;
    long v = 0;
    if (ac > 2 && !parse_val(av[2], &mode, &v)) { say("valor inválido: %s\n", av[2]); return 1; }
    run_jobs(mask, code, mode, v, pct);
    return report(mask, mode, code, pct);
}

static int cmd_vcp(int ac, char **av)
{
    if (ac < 3) { sayl("uso: vcp <mon> <código> [valor]"); return 1; }
    char *e;
    long code = strtol(av[2], &e, 0);
    if (*e || code < 0 || code > 255) { say("código inválido: %s\n", av[2]); return 1; }
    char *nav[3] = { av[0], av[1], ac > 3 ? av[3] : NULL };
    return cmd_feature(ac > 3 ? 3 : 2, nav, (BYTE)code, 0);
}

static int cmd_input(int ac, char **av)
{
    if (ac < 2) { sayl("uso: input <mon> [hdmi1|hdmi2|dp1|dp2|usbc|dvi1|vga1|0xNN]"); return 1; }
    unsigned mask = targets(av[1]);
    if (!mask) return bad_target(av[1]);
    if (ac < 3) { run_jobs(mask, 0x60, J_GET, 0, 0); return report(mask, J_GET, 0x60, 0); }
    long v = -1;
    for (size_t i = 0; i < sizeof inputs / sizeof *inputs; i++)
        if (!_stricmp(av[2], inputs[i].n)) v = inputs[i].v;
    if (v < 0) {
        char *e;
        v = strtol(av[2], &e, 0);
        if (*e || v <= 0) { say("fuente inválida: %s\n", av[2]); return 1; }
    }
    run_jobs(mask, 0x60, J_SET, v, 0);
    return report(mask, J_SET, 0x60, 0);
}

static int cmd_power(int ac, char **av)
{
    if (ac < 3) { sayl("uso: power <mon> on|standby|off|hardoff"); return 1; }
    unsigned mask = targets(av[1]);
    if (!mask) return bad_target(av[1]);
    static const char *names[] = { "on", "standby", "suspend", "off", "hardoff" };
    for (int i = 0; i < 5; i++)
        if (!_stricmp(av[2], names[i])) {
            run_jobs(mask, 0xD6, J_SET, i + 1, 0);
            return report(mask, J_SET, 0xD6, 0);
        }
    say("estado inválido: %s\n", av[2]);
    return 1;
}

/* ---------- configuración de pantalla (GDI / CCD) ---------- */

static int disp_err(const char *dev, LONG r)
{
    if (r == DISP_CHANGE_SUCCESSFUL) return 0;
    const char *s = r == DISP_CHANGE_BADMODE ? "modo no soportado"
                  : r == DISP_CHANGE_RESTART ? "requiere reiniciar"
                  : r == DISP_CHANGE_BADPARAM ? "parámetro inválido"
                  : "fallo al aplicar";
    say("%s: %s (%ld)\n", dev ? dev : "pantalla", s, r);
    return 1;
}

static int apply_all(void)
{
    return disp_err(NULL, ChangeDisplaySettingsExA(NULL, NULL, NULL, 0, NULL));
}

static int cur_mode(int i, DEVMODEA *dm)
{
    memset(dm, 0, sizeof *dm);
    dm->dmSize = sizeof *dm;
    return EnumDisplaySettingsA(mon[i].dev, ENUM_CURRENT_SETTINGS, dm);
}

static int cmd_mode(int ac, char **av)
{
    if (ac < 3) { sayl("uso: mode <mon> AxB[@Hz] | @Hz"); return 1; }
    unsigned mask = targets(av[1]);
    if (!mask) return bad_target(av[1]);
    DWORD w = 0, h = 0, hz = 0;
    const char *at = strchr(av[2], '@');
    if (av[2][0] != '@' && sscanf(av[2], "%lux%lu", &w, &h) != 2) { say("modo inválido: %s\n", av[2]); return 1; }
    if (at) hz = strtoul(at + 1, NULL, 10);
    int rc = 0;
    for (int i = 0; i < nmon; i++) {
        if (!(mask & (1u << i))) continue;
        DEVMODEA dm;
        cur_mode(i, &dm);
        dm.dmFields = 0;
        if (w) {
            int rot = dm.dmDisplayOrientation & 1;
            dm.dmPelsWidth = rot ? h : w;
            dm.dmPelsHeight = rot ? w : h;
            dm.dmFields |= DM_PELSWIDTH | DM_PELSHEIGHT;
        }
        if (hz) { dm.dmDisplayFrequency = hz; dm.dmFields |= DM_DISPLAYFREQUENCY; }
        rc |= disp_err(mon[i].dev, ChangeDisplaySettingsExA(mon[i].dev, &dm, NULL, CDS_UPDATEREGISTRY | CDS_NORESET, NULL));
    }
    return apply_all() | rc;
}

typedef struct { DWORD w, h, hz; } Res;
static int rescmp(const void *a, const void *b)
{
    const Res *x = a, *y = b;
    if (x->w != y->w) return x->w < y->w ? 1 : -1;
    if (x->h != y->h) return x->h < y->h ? 1 : -1;
    return x->hz < y->hz ? 1 : x->hz > y->hz ? -1 : 0;
}

/* Modos de 32 bpp únicos, ordenados de mayor a menor (malloc). */
static int collect_modes(int i, Res **out)
{
    Res *r = NULL;
    int n = 0, cap = 0;
    DEVMODEA dm = { .dmSize = sizeof dm };
    for (DWORD k = 0; EnumDisplaySettingsA(mon[i].dev, k, &dm); k++) {
        if (dm.dmBitsPerPel != 32) continue;
        int dup = 0;
        for (int q = 0; q < n && !dup; q++)
            dup = r[q].w == dm.dmPelsWidth && r[q].h == dm.dmPelsHeight && r[q].hz == dm.dmDisplayFrequency;
        if (dup) continue;
        if (n == cap) r = realloc(r, (cap = cap ? cap * 2 : 64) * sizeof *r);
        r[n++] = (Res){ dm.dmPelsWidth, dm.dmPelsHeight, dm.dmDisplayFrequency };
    }
    if (n) qsort(r, n, sizeof *r, rescmp);
    *out = r;
    return n;
}

static int cmd_modes(int ac, char **av)
{
    int i = one_target(ac > 1 ? av[1] : "p");
    if (i < 0) return bad_target(av[1]);
    Res *r;
    int n = collect_modes(i, &r);
    for (int q = 0; q < n; q++) {
        if (!q || r[q].w != r[q - 1].w || r[q].h != r[q - 1].h) say("%s%lux%lu @", q ? "\n" : "", r[q].w, r[q].h);
        say(" %lu", r[q].hz);
    }
    sayl("");
    free(r);
    return 0;
}

static int cmd_rot(int ac, char **av)
{
    if (ac < 3) { sayl("uso: rot <mon> 0|90|180|270"); return 1; }
    unsigned mask = targets(av[1]);
    if (!mask) return bad_target(av[1]);
    int deg = atoi(av[2]);
    if (deg % 90 || deg < 0 || deg > 270) { say("ángulo inválido: %s\n", av[2]); return 1; }
    int rc = 0;
    for (int i = 0; i < nmon; i++) {
        if (!(mask & (1u << i))) continue;
        DEVMODEA dm;
        cur_mode(i, &dm);
        DWORD o = deg / 90;
        if ((o & 1) != (dm.dmDisplayOrientation & 1)) {
            DWORD t = dm.dmPelsWidth;
            dm.dmPelsWidth = dm.dmPelsHeight;
            dm.dmPelsHeight = t;
        }
        dm.dmDisplayOrientation = o;
        dm.dmFields = DM_DISPLAYORIENTATION | DM_PELSWIDTH | DM_PELSHEIGHT;
        rc |= disp_err(mon[i].dev, ChangeDisplaySettingsExA(mon[i].dev, &dm, NULL, CDS_UPDATEREGISTRY | CDS_NORESET, NULL));
    }
    return apply_all() | rc;
}

static int cmd_primary(int ac, char **av)
{
    int t = one_target(ac > 1 ? av[1] : NULL);
    if (t < 0) return bad_target(ac > 1 ? av[1] : NULL);
    if (mon[t].primary) return 0;
    DEVMODEA dm[MAXMON];
    for (int i = 0; i < nmon; i++) cur_mode(i, &dm[i]);
    LONG px = dm[t].dmPosition.x, py = dm[t].dmPosition.y;
    int rc = 0;
    for (int k = 0; k < nmon; k++) {
        int i = (k + t) % nmon; /* el nuevo principal primero */
        dm[i].dmPosition.x -= px;
        dm[i].dmPosition.y -= py;
        dm[i].dmFields = DM_POSITION;
        DWORD fl = CDS_UPDATEREGISTRY | CDS_NORESET | (i == t ? CDS_SET_PRIMARY : 0);
        rc |= disp_err(mon[i].dev, ChangeDisplaySettingsExA(mon[i].dev, &dm[i], NULL, fl, NULL));
    }
    return apply_all() | rc;
}

static int cmd_disable(int ac, char **av)
{
    unsigned mask = targets(ac > 1 ? av[1] : NULL);
    if (!mask) return bad_target(ac > 1 ? av[1] : NULL);
    int rc = 0;
    for (int i = 0; i < nmon; i++) {
        if (!(mask & (1u << i))) continue;
        if (mon[i].primary) { say("%d: es el principal, cambia primero con 'primary'\n", i + 1); rc = 1; continue; }
        DEVMODEA dm = { .dmSize = sizeof dm, .dmFields = DM_POSITION | DM_PELSWIDTH | DM_PELSHEIGHT };
        rc |= disp_err(mon[i].dev, ChangeDisplaySettingsExA(mon[i].dev, &dm, NULL, CDS_UPDATEREGISTRY | CDS_NORESET, NULL));
    }
    return apply_all() | rc;
}

typedef struct { DISPLAYCONFIG_PATH_INFO *p; DISPLAYCONFIG_MODE_INFO *m; UINT32 np, nm; } Cfg;

static int cfg_query(Cfg *c, UINT32 flags)
{
    memset(c, 0, sizeof *c);
    if (GetDisplayConfigBufferSizes(flags, &c->np, &c->nm)) return 0;
    c->p = calloc(c->np ? c->np : 1, sizeof *c->p);
    c->m = calloc(c->nm ? c->nm : 1, sizeof *c->m);
    if (QueryDisplayConfig(flags, &c->np, c->p, &c->nm, c->m, NULL)) {
        free(c->p);
        free(c->m);
        return 0;
    }
    return 1;
}

static int same_target(const DISPLAYCONFIG_PATH_INFO *a, const DISPLAYCONFIG_PATH_INFO *b)
{
    return a->targetInfo.id == b->targetInfo.id && a->targetInfo.adapterId.LowPart == b->targetInfo.adapterId.LowPart &&
           a->targetInfo.adapterId.HighPart == b->targetInfo.adapterId.HighPart;
}

static int same_source(const DISPLAYCONFIG_PATH_INFO *a, const DISPLAYCONFIG_PATH_INFO *b)
{
    return a->sourceInfo.id == b->sourceInfo.id && a->sourceInfo.adapterId.LowPart == b->sourceInfo.adapterId.LowPart &&
           a->sourceInfo.adapterId.HighPart == b->sourceInfo.adapterId.HighPart;
}

/* Monitores conectados sin ruta activa, y si alguna fuente está clonada en varios monitores. */
static void topo_status(int *missing, int *cloned)
{
    Cfg a;
    *missing = *cloned = 0;
    if (!cfg_query(&a, QDC_ALL_PATHS)) return;
    for (UINT32 i = 0; i < a.np; i++) {
        DISPLAYCONFIG_PATH_INFO *p = &a.p[i];
        int dup = 0, active = 0;
        for (UINT32 j = 0; j < i && !dup; j++) dup = same_target(&a.p[j], p);
        if (dup || !p->targetInfo.targetAvailable) continue;
        for (UINT32 j = 0; j < a.np && !active; j++)
            active = (a.p[j].flags & DISPLAYCONFIG_PATH_ACTIVE) && same_target(&a.p[j], p);
        if (!active) (*missing)++;
    }
    for (UINT32 i = 0; i < a.np; i++)
        for (UINT32 j = i + 1; j < a.np; j++)
            if ((a.p[i].flags & a.p[j].flags & DISPLAYCONFIG_PATH_ACTIVE) && same_source(&a.p[i], &a.p[j])) *cloned = 1;
    free(a.p);
    free(a.m);
}

/* Activa cada monitor conectado que esté apagado, con una fuente libre; Windows le asigna modo y posición. */
static LONG activate_missing(void)
{
    Cfg a;
    if (!cfg_query(&a, QDC_ALL_PATHS)) return -1;
    DISPLAYCONFIG_PATH_INFO *sel = calloc(a.np ? a.np : 1, sizeof *sel);
    UINT32 n = 0;
    for (UINT32 i = 0; i < a.np; i++)
        if (a.p[i].flags & DISPLAYCONFIG_PATH_ACTIVE) sel[n++] = a.p[i];
    for (UINT32 i = 0; i < a.np; i++) {
        DISPLAYCONFIG_PATH_INFO p = a.p[i];
        if ((p.flags & DISPLAYCONFIG_PATH_ACTIVE) || !p.targetInfo.targetAvailable) continue;
        int used = 0;
        for (UINT32 k = 0; k < n && !used; k++) used = same_target(&sel[k], &p) || same_source(&sel[k], &p);
        if (used) continue;
        p.flags = DISPLAYCONFIG_PATH_ACTIVE;
        p.sourceInfo.modeInfoIdx = DISPLAYCONFIG_PATH_MODE_IDX_INVALID;
        p.targetInfo.modeInfoIdx = DISPLAYCONFIG_PATH_MODE_IDX_INVALID;
        sel[n++] = p;
    }
    /* Primero sólo las rutas: Windows recupera de su base de datos el modo y la posición que
     * recordaba para esta combinación de monitores. Si no la conoce, se suministran los modos
     * actuales y Windows elige valores para los monitores nuevos. */
    DISPLAYCONFIG_PATH_INFO *bare = calloc(n ? n : 1, sizeof *bare);
    for (UINT32 k = 0; k < n; k++) {
        bare[k] = sel[k];
        bare[k].sourceInfo.modeInfoIdx = DISPLAYCONFIG_PATH_MODE_IDX_INVALID;
        bare[k].targetInfo.modeInfoIdx = DISPLAYCONFIG_PATH_MODE_IDX_INVALID;
    }
    LONG r = SetDisplayConfig(n, bare, 0, NULL, SDC_APPLY | SDC_TOPOLOGY_SUPPLIED | SDC_ALLOW_PATH_ORDER_CHANGES);
    free(bare);
    if (r)
        r = SetDisplayConfig(n, sel, a.nm, a.m, SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_ALLOW_CHANGES | SDC_SAVE_TO_DATABASE);
    free(sel);
    free(a.p);
    free(a.m);
    return r;
}

/* La topología "extender" de Windows reutiliza la última guardada, que puede excluir monitores:
 * tras aplicarla se activan los monitores conectados que hayan quedado apagados. */
static int topo_extend(void)
{
    int missing, cloned;
    topo_status(&missing, &cloned);
    if (!missing && !cloned) return sayl("las pantallas ya están extendidas"), 0;
    if (cloned) SetDisplayConfig(0, NULL, 0, NULL, SDC_APPLY | SDC_TOPOLOGY_EXTEND);
    topo_status(&missing, &cloned);
    if (!missing) return 0;
    LONG r = activate_missing();
    if (r) say("no se pudieron activar todos los monitores (error %ld)\n", r);
    return !!r;
}

/* Igual que con "extender", la topología "duplicar" guardada puede dejar monitores fuera:
 * se conectan explícitamente todos los monitores del adaptador a una misma fuente. */
static int topo_clone(void)
{
    Cfg a;
    if (!cfg_query(&a, QDC_ALL_PATHS)) return sayl("topo: error al consultar"), 1;
    DISPLAYCONFIG_PATH_INFO *src = NULL;
    for (UINT32 i = 0; i < a.np && !src; i++)
        if (a.p[i].flags & DISPLAYCONFIG_PATH_ACTIVE) src = &a.p[i];
    if (!src) { free(a.p); free(a.m); return sayl("no hay ninguna pantalla activa"), 1; }

    DISPLAYCONFIG_PATH_INFO *sel = calloc(a.np, sizeof *sel);
    UINT32 n = 0;
    int avail = 0;
    for (UINT32 i = 0; i < a.np; i++) {
        DISPLAYCONFIG_PATH_INFO p = a.p[i];
        if (!p.targetInfo.targetAvailable) continue;
        int dup = 0;
        for (UINT32 j = 0; j < i && !dup; j++) dup = same_target(&a.p[j], &p);
        if (!dup) avail++;
        if (!same_source(&p, src)) continue;
        int used = 0;
        for (UINT32 k = 0; k < n && !used; k++) used = same_target(&sel[k], &p);
        if (used) continue;
        p.flags = DISPLAYCONFIG_PATH_ACTIVE;
        p.sourceInfo.modeInfoIdx = DISPLAYCONFIG_PATH_MODE_IDX_INVALID;
        p.targetInfo.modeInfoIdx = DISPLAYCONFIG_PATH_MODE_IDX_INVALID;
        sel[n++] = p;
    }
    LONG r = SetDisplayConfig(n, sel, 0, NULL, SDC_APPLY | SDC_TOPOLOGY_SUPPLIED | SDC_ALLOW_PATH_ORDER_CHANGES);
    if (r)
        r = SetDisplayConfig(n, sel, a.nm, a.m, SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_ALLOW_CHANGES | SDC_SAVE_TO_DATABASE);
    if (r) say("no se pudo duplicar en %u pantallas (error %ld)\n", n, r);
    else if ((int)n < avail) say("duplicado en %u de %d pantallas (el resto está en otro adaptador)\n", n, avail);
    free(sel);
    free(a.p);
    free(a.m);
    return !!r;
}

static int cmd_topo(int ac, char **av)
{
    static const struct { const char *n; UINT32 f; } t[] = {
        { "extend", SDC_TOPOLOGY_EXTEND }, { "clone", SDC_TOPOLOGY_CLONE },
        { "internal", SDC_TOPOLOGY_INTERNAL }, { "external", SDC_TOPOLOGY_EXTERNAL },
    };
    if (ac > 1 && !_stricmp(av[1], "extend")) return topo_extend();
    if (ac > 1 && !_stricmp(av[1], "clone")) return topo_clone();
    for (int i = 0; ac > 1 && i < 4; i++)
        if (!_stricmp(av[1], t[i].n)) {
            LONG r = SetDisplayConfig(0, NULL, 0, NULL, SDC_APPLY | t[i].f);
            if (r) say("topo: error %ld\n", r);
            return !!r;
        }
    sayl("uso: topo extend|clone|internal|external");
    return 1;
}

/* layout save/load: guarda las rutas activas de CCD y las restaura (re-mapeando LUIDs tras reinicio). */
static int layout_name_ok(const char *n)
{
    if (!*n) return 0;
    for (const char *p = n; *p; p++)
        if (strchr("\\/:*?\"<>|", *p)) return 0;
    return 1;
}

static void layout_path(char *out, const char *name)
{
    snprintf(out, MAX_PATH, "%s\\layouts", exedir);
    CreateDirectoryA(out, NULL);
    snprintf(out, MAX_PATH, "%s\\layouts\\%s.bin", exedir, name);
}

#define LAYOUT_MAGIC 0x5453414Bu

void (*link_layout_hook)(const char *op, const char *path);

/* Sesiones de Kast Link de la disposición (layouts\<nombre>.link, lo escribe y lee el agente). */
static void link_layout_path(char *out, const char *name)
{
    snprintf(out, MAX_PATH, "%s\\layouts\\%s.link", exedir, name);
}

static void layout_link(const char *op, const char *name)
{
    char path[MAX_PATH];
    link_layout_path(path, name);
    if (link_layout_hook) { link_layout_hook(op, path); return; }
    HWND h = find_daemon();
    if (!h) return;
    char buf[MAX_PATH + 16];
    int n = snprintf(buf, sizeof buf, "%s\n%s", op, path);
    COPYDATASTRUCT cd = { KAST_COPY_LINK_LAYOUT, (DWORD)n + 1, buf };
    SendMessageTimeoutA(h, WM_COPYDATA, 0, (LPARAM)&cd, SMTO_ABORTIFHUNG, 2000, NULL);
}

static int layout_save(const char *name)
{
    UINT32 np = 0, nm = 0;
    if (GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &np, &nm)) return sayl("layout: error al consultar"), 1;
    DISPLAYCONFIG_PATH_INFO *p = calloc(np, sizeof *p);
    DISPLAYCONFIG_MODE_INFO *m = calloc(nm, sizeof *m);
    int rc = 1;
    if (!QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &np, p, &nm, m, NULL)) {
        char path[MAX_PATH];
        layout_path(path, name);
        FILE *f = fopen(path, "wb");
        if (f) {
            UINT32 hdr[3] = { LAYOUT_MAGIC, np, nm };
            fwrite(hdr, sizeof hdr, 1, f);
            fwrite(p, sizeof *p, np, f);
            fwrite(m, sizeof *m, nm, f);
            fclose(f);
            say("layout '%s' guardado\n", name);
            layout_link("save", name);
            rc = 0;
        }
    }
    free(p);
    free(m);
    return rc;
}

static int luid_eq(LUID a, LUID b) { return a.LowPart == b.LowPart && a.HighPart == b.HighPart; }

static int layout_load(const char *name)
{
    char path[MAX_PATH];
    layout_path(path, name);
    FILE *f = fopen(path, "rb");
    if (!f) return say("layout '%s' no existe\n", name), 1;
    UINT32 hdr[3] = { 0 };
    if (fread(hdr, sizeof hdr, 1, f) != 1 || hdr[0] != LAYOUT_MAGIC || hdr[1] > 256 || hdr[2] > 512) {
        fclose(f);
        return sayl("layout corrupto"), 1;
    }
    UINT32 np = hdr[1], nm = hdr[2];
    DISPLAYCONFIG_PATH_INFO *p = calloc(np, sizeof *p);
    DISPLAYCONFIG_MODE_INFO *m = calloc(nm, sizeof *m);
    fread(p, sizeof *p, np, f);
    fread(m, sizeof *m, nm, f);
    fclose(f);

    UINT32 cp = 0, cm = 0;
    GetDisplayConfigBufferSizes(QDC_ALL_PATHS, &cp, &cm);
    DISPLAYCONFIG_PATH_INFO *P = calloc(cp, sizeof *P);
    DISPLAYCONFIG_MODE_INFO *M = calloc(cm, sizeof *M);
    LUID from[8], to[8];
    int nl = 0;
    if (!QueryDisplayConfig(QDC_ALL_PATHS, &cp, P, &cm, M, NULL))
        for (UINT32 i = 0; i < np; i++)
            for (UINT32 j = 0; j < cp; j++)
                if (P[j].sourceInfo.id == p[i].sourceInfo.id && P[j].targetInfo.id == p[i].targetInfo.id) {
                    int known = 0;
                    for (int q = 0; q < nl; q++) known |= luid_eq(from[q], p[i].targetInfo.adapterId);
                    if (!known && nl < 8) { from[nl] = p[i].targetInfo.adapterId; to[nl++] = P[j].targetInfo.adapterId; }
                    break;
                }
    for (int q = 0; q < nl; q++) {
        for (UINT32 i = 0; i < np; i++) {
            if (luid_eq(p[i].sourceInfo.adapterId, from[q])) p[i].sourceInfo.adapterId = to[q];
            if (luid_eq(p[i].targetInfo.adapterId, from[q])) p[i].targetInfo.adapterId = to[q];
        }
        for (UINT32 i = 0; i < nm; i++)
            if (luid_eq(m[i].adapterId, from[q])) m[i].adapterId = to[q];
    }
    LONG r = SetDisplayConfig(np, p, nm, m, SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_ALLOW_CHANGES | SDC_SAVE_TO_DATABASE);
    if (r) say("layout: error %ld\n", r);
    free(p); free(m); free(P); free(M);
    layout_link("load", name);
    return !!r;
}

static void layout_each(void (*fn)(const char *, void *), void *ctx)
{
    char pat[MAX_PATH];
    snprintf(pat, sizeof pat, "%s\\layouts\\*.bin", exedir);
    WIN32_FIND_DATAA fd;
    HANDLE h = FindFirstFileA(pat, &fd);
    if (h == INVALID_HANDLE_VALUE) return;
    do {
        char *dot = strrchr(fd.cFileName, '.');
        if (dot) *dot = 0;
        fn(fd.cFileName, ctx);
    } while (FindNextFileA(h, &fd));
    FindClose(h);
}

static void layout_print(const char *n, void *ctx) { (void)ctx; sayl(n); }

static int cmd_layout(int ac, char **av)
{
    if (ac >= 3 && !layout_name_ok(av[2])) return say("nombre inválido: %s\n", av[2]), 1;
    if (ac >= 3 && !_stricmp(av[1], "save")) return layout_save(av[2]);
    if (ac >= 3 && !_stricmp(av[1], "load")) return layout_load(av[2]);
    if (ac >= 3 && !_stricmp(av[1], "del")) {
        char path[MAX_PATH];
        layout_path(path, av[2]);
        int ok = DeleteFileA(path);
        link_layout_path(path, av[2]);
        DeleteFileA(path);
        return !ok;
    }
    if (ac >= 2 && _stricmp(av[1], "list")) { sayl("uso: layout [list] | layout save|load|del <nombre>"); return 1; }
    layout_each(layout_print, NULL);
    return 0;
}

/* ---------- configuración (kast.ini) ---------- */

static const struct { const char *n; UINT vk; } keys[] = {
    { "up", VK_UP }, { "down", VK_DOWN }, { "left", VK_LEFT }, { "right", VK_RIGHT },
    { "pgup", VK_PRIOR }, { "pgdn", VK_NEXT }, { "home", VK_HOME }, { "end", VK_END },
    { "ins", VK_INSERT }, { "del", VK_DELETE }, { "space", VK_SPACE }, { "tab", VK_TAB },
    { "esc", VK_ESCAPE }, { "enter", VK_RETURN }, { "pause", VK_PAUSE }, { "scroll", VK_SCROLL },
    { "plus", VK_OEM_PLUS }, { "minus", VK_OEM_MINUS }, { "comma", VK_OEM_COMMA }, { "period", VK_OEM_PERIOD },
    { "numplus", VK_ADD }, { "numminus", VK_SUBTRACT }, { "nummul", VK_MULTIPLY }, { "numdiv", VK_DIVIDE },
};

static char *trim(char *s)
{
    while (isspace((unsigned char)*s)) s++;
    char *e = s + strlen(s);
    while (e > s && isspace((unsigned char)e[-1])) *--e = 0;
    return s;
}

static int parse_combo(const char *combo, UINT *mods, UINT *vk)
{
    char buf[128];
    snprintf(buf, sizeof buf, "%s", combo);
    *mods = MOD_NOREPEAT;
    *vk = 0;
    for (char *t = buf, *next; t; t = next) {
        next = strchr(t, '+');
        if (next) *next++ = 0;
        t = trim(t);
        if (!_stricmp(t, "ctrl") || !_stricmp(t, "control")) *mods |= MOD_CONTROL;
        else if (!_stricmp(t, "alt")) *mods |= MOD_ALT;
        else if (!_stricmp(t, "shift")) *mods |= MOD_SHIFT;
        else if (!_stricmp(t, "win")) *mods |= MOD_WIN;
        else if (strlen(t) == 1 && isalnum((unsigned char)*t)) *vk = toupper((unsigned char)*t);
        else if ((t[0] == 'f' || t[0] == 'F') && atoi(t + 1) >= 1 && atoi(t + 1) <= 24) *vk = VK_F1 + atoi(t + 1) - 1;
        else if (!_strnicmp(t, "num", 3) && isdigit((unsigned char)t[3]) && !t[4]) *vk = VK_NUMPAD0 + t[3] - '0';
        else if (!_strnicmp(t, "0x", 2)) *vk = strtoul(t, NULL, 16);
        else {
            size_t i;
            for (i = 0; i < sizeof keys / sizeof *keys; i++)
                if (!_stricmp(t, keys[i].n)) { *vk = keys[i].vk; break; }
            if (i == sizeof keys / sizeof *keys) return 0;
        }
    }
    return *vk != 0;
}

void ini_path(char *out) { snprintf(out, MAX_PATH, "%s\\kast.ini", exedir); }

/* Secciones: [favoritos] nombre = comandos ; [atajos] combo = comandos (sin sección = atajos). */
void load_config(Sb *log)
{
    EnterCriticalSection(&cfg_cs);
    for (int i = 0; i < nfav; i++) { free(favs[i].name); free(favs[i].act); }
    for (int i = 0; i < nhk; i++) { free(hks[i].combo); free(hks[i].act); }
    nfav = nhk = ncal = 0;
    cfg_loaded = 1;
    char path[MAX_PATH];
    ini_path(path);
    FILE *f = fopen(path, "r");
    if (!f) {
        if (log) sb_printf(log, "no se encontró %s\n", path);
        LeaveCriticalSection(&cfg_cs);
        return;
    }
    char line[1024];
    int ln = 0, sec = 0;
    while (fgets(line, sizeof line, f)) {
        ln++;
        char *s = line;
        if (ln == 1 && !strncmp(s, "\xEF\xBB\xBF", 3)) s += 3;
        s = trim(s);
        if (!*s || *s == '#' || *s == ';') continue;
        if (*s == '[') {
            sec = !_strnicmp(s, "[favoritos]", 11) || !_strnicmp(s, "[favorites]", 11) ? 1
                : !_strnicmp(s, "[atajos]", 8) || !_strnicmp(s, "[hotkeys]", 9) ? 0
                : !_strnicmp(s, "[calibracion]", 13) || !_strnicmp(s, "[calibration]", 13) ? 3
                : !_strnicmp(s, "[calibracion-contraste]", 23) || !_strnicmp(s, "[calibration-contrast]", 22) ? 4 : 2;
            continue;
        }
        char *eq = strchr(s, '=');
        if (!eq) { if (log) sb_printf(log, "línea %d: falta '='\n", ln); continue; }
        *eq = 0;
        char *key = trim(s), *act = trim(eq + 1);
        if (sec == 3 || sec == 4) {
            Curve cv;
            Cal *c = parse_curve(act, &cv) ? cal_slot(key) : NULL;
            if (c) *(sec == 3 ? &c->b : &c->c) = cv;
            else if (log) sb_printf(log, "línea %d: calibración inválida (usa: monitor = 0:mínimo 50:medio 100:máximo)\n", ln);
        } else if (sec == 1 && nfav < MAXFAV) {
            favs[nfav++] = (Fav){ _strdup(key), _strdup(act) };
        } else if (sec == 0 && nhk < MAXHK) {
            UINT mods, vk;
            if (!parse_combo(key, &mods, &vk)) { if (log) sb_printf(log, "línea %d: tecla inválida '%s'\n", ln, key); continue; }
            hks[nhk++] = (Hotkey){ mods, vk, _strdup(key), _strdup(act) };
        }
    }
    fclose(f);
    LeaveCriticalSection(&cfg_cs);
}

static int cmd_fav(int ac, char **av)
{
    if (!cfg_loaded) load_config(NULL);
    char name[256] = "";
    for (int i = 1; i < ac; i++) {
        strncat(name, av[i], sizeof name - strlen(name) - 2);
        if (i + 1 < ac) strcat(name, " ");
    }
    char *act = NULL;
    EnterCriticalSection(&cfg_cs);
    for (int i = 0; i < nfav; i++) {
        if (!*name) say("%s = %s\n", favs[i].name, favs[i].act);
        else if (!_stricmp(favs[i].name, name)) act = _strdup(favs[i].act);
    }
    LeaveCriticalSection(&cfg_cs);
    if (!*name) return 0;
    if (!act) return say("favorito no encontrado: %s\n", name), 1;
    if (fav_depth > 4) { free(act); return sayl("favoritos anidados demasiado profundo"), 1; }
    fav_depth++;
    int rc = exec_line(act);
    fav_depth--;
    free(act);
    return rc;
}

int autostart_get(void)
{
    return !RegGetValueA(HKEY_CURRENT_USER, "Software\\Microsoft\\Windows\\CurrentVersion\\Run", "Kast",
                         RRF_RT_REG_SZ, NULL, NULL, NULL);
}

int autostart_set(int on)
{
    HKEY k;
    if (RegOpenKeyExA(HKEY_CURRENT_USER, "Software\\Microsoft\\Windows\\CurrentVersion\\Run", 0, KEY_SET_VALUE, &k))
        return 1;
    LONG r;
    if (on) {
        char v[MAX_PATH + 16];
        snprintf(v, sizeof v, "\"%s\\kastd.exe\"", exedir);
        r = RegSetValueExA(k, "Kast", 0, REG_SZ, (BYTE *)v, (DWORD)strlen(v) + 1);
    } else {
        r = RegDeleteValueA(k, "Kast");
        if (r == ERROR_FILE_NOT_FOUND) r = 0;
    }
    RegCloseKey(k);
    return !!r;
}

/* ---------- estado para la UI ---------- */

typedef struct { int idx; Curve cv[2]; DWORD v[3], raw[2]; int ok[3]; } St;
static const BYTE st_codes[3] = { 0x10, 0x12, 0x60 };

static DWORD WINAPI st_run(LPVOID arg)
{
    St *s = arg;
    Mon *m = &mon[s->idx];
    for (int c = 0; c < 3; c++) {
        DWORD cur = 0, max = 0;
        if (!ddc_get(m, 0, st_codes[c], &cur, &max)) continue;
        cache_put(m->dev, 0, st_codes[c], cur, max);
        DWORD pct = max ? (cur * 100 + max / 2) / max : cur;
        if (c < 2) {
            s->raw[c] = pct;
            pct = (DWORD)to_user((long)pct, &s->cv[c]);
        }
        s->v[c] = c < 2 ? pct : (cur & 0xFF);
        s->ok[c] = 1;
    }
    return 0;
}

static void layout_json(const char *n, void *ctx)
{
    Sb *s = ctx;
    if (s->p[s->n - 1] != '[') sb_add(s, ",", 1);
    sb_json(s, n);
}

/* "nombre":[["Mac mini",1,3],…]: dispositivo, su pantalla y el monitor local de cada sesión guardada. */
static void layout_links_json(const char *n, void *ctx)
{
    Sb *s = ctx;
    char path[MAX_PATH], line[512];
    link_layout_path(path, n);
    FILE *f = fopen(path, "rb");
    if (!f) return;
    if (s->p[s->n - 1] != '{') sb_add(s, ",", 1);
    sb_json(s, n);
    sb_add(s, ":[", 2);
    int first = 1;
    while (fgets(line, sizeof line, f)) {
        /* id \t nombre \t su pantalla \t mi monitor \t control */
        char *fld[5] = { 0 }, *p = line;
        int k = 0;
        while (k < 5 && p) { fld[k++] = p; p = strchr(p, '\t'); if (p) *p++ = 0; }
        if (k < 4) continue;
        sb_add(s, first ? "[" : ",[", first ? 1 : 2);
        first = 0;
        sb_json(s, fld[1]);
        sb_printf(s, ",%d,%d]", atoi(fld[2]), atoi(fld[3]));
    }
    fclose(f);
    sb_add(s, "]", 1);
}

char *state_json(const char *log)
{
    enum_mons();
    St st[MAXMON] = { 0 };
    HANDLE th[MAXMON];
    int nt = 0;
    for (int i = 0; i < nmon; i++) {
        st[i].idx = i;
        cal_get(mon[i].name, 0x10, &st[i].cv[0]);
        cal_get(mon[i].name, 0x12, &st[i].cv[1]);
        if (!phys(&mon[i])) continue;
        th[nt] = CreateThread(NULL, 64 * 1024, st_run, &st[i], STACK_SIZE_PARAM_IS_A_RESERVATION, NULL);
        if (th[nt]) nt++;
    }
    if (nt) WaitForMultipleObjects(nt, th, TRUE, INFINITE);
    for (int i = 0; i < nt; i++) CloseHandle(th[i]);

    Sb s = { 0 };
    sb_printf(&s, "{\"t\":\"state\",\"mons\":[");
    for (int i = 0; i < nmon; i++) {
        DEVMODEA dm;
        cur_mode(i, &dm);
        sb_printf(&s, "%s{\"i\":%d,\"name\":", i ? "," : "", i + 1);
        sb_json(&s, mon[i].name);
        sb_printf(&s, ",\"dev\":");
        sb_json(&s, mon[i].dev);
        sb_printf(&s, ",\"primary\":%s,\"x\":%ld,\"y\":%ld,\"w\":%ld,\"h\":%ld,\"pw\":%lu,\"ph\":%lu,\"hz\":%lu,\"rot\":%lu",
                  mon[i].primary ? "true" : "false", mon[i].rc.left, mon[i].rc.top,
                  mon[i].rc.right - mon[i].rc.left, mon[i].rc.bottom - mon[i].rc.top,
                  dm.dmPelsWidth, dm.dmPelsHeight, dm.dmDisplayFrequency, dm.dmDisplayOrientation * 90);
        static const char *keys_[3] = { "b", "c", "in" };
        for (int c = 0; c < 3; c++)
            if (st[i].ok[c]) sb_printf(&s, ",\"%s\":%lu", keys_[c], st[i].v[c]);
            else sb_printf(&s, ",\"%s\":null", keys_[c]);
        sb_printf(&s, ",\"braw\":%lu,\"craw\":%lu", st[i].raw[0], st[i].raw[1]);
        sb_printf(&s, ",\"modes\":[");
        Res *r;
        int n = collect_modes(i, &r);
        for (int q = 0; q < n; q++) sb_printf(&s, "%s\"%lux%lu@%lu\"", q ? "," : "", r[q].w, r[q].h, r[q].hz);
        free(r);
        sb_printf(&s, "]}");
    }
    sb_printf(&s, "],\"layouts\":[");
    layout_each(layout_json, &s);
    sb_printf(&s, "],\"layoutLinks\":{");
    layout_each(layout_links_json, &s);
    sb_printf(&s, "}");
    BOOL vanish = FALSE;
    SystemParametersInfoW(SPI_GETMOUSEVANISH, 0, &vanish, 0);
    sb_printf(&s, ",\"autostart\":%s,\"vanish\":%s,\"dir\":", autostart_get() ? "true" : "false", vanish ? "true" : "false");
    sb_json(&s, exedir);
    sb_printf(&s, ",\"log\":");
    sb_json(&s, log);
    sb_printf(&s, ",\"ini\":");
    char path[MAX_PATH];
    ini_path(path);
    Sb ini = { 0 };
    FILE *f = fopen(path, "rb");
    if (f) {
        char buf[4096];
        size_t n;
        while ((n = fread(buf, 1, sizeof buf, f))) sb_add(&ini, buf, n);
        fclose(f);
    }
    sb_json(&s, ini.p && !strncmp(ini.p, "\xEF\xBB\xBF", 3) ? ini.p + 3 : ini.p);
    free(ini.p);
    sb_add(&s, "}", 1);
    return s.p;
}

/* ---------- varios ---------- */

static void wait_keys_released(void)
{
    static const int vk[] = { VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN, VK_RETURN, VK_LBUTTON };
    for (int t = 0; t < 40; t++) {
        int down = 0;
        for (size_t i = 0; i < sizeof vk / sizeof *vk; i++) down |= GetAsyncKeyState(vk[i]) & 0x8000;
        if (!down) break;
        Sleep(50);
    }
    Sleep(250);
}

static int cmd_list(void)
{
    if (!nmon) return sayl("sin monitores"), 1;
    run_jobs((1u << nmon) - 1, 0x10, J_GET, 0, 1);
    say("#   %-20s %-16s %-12s %-8s %s\n", "Monitor", "Modo", "Posición", "Brillo", "Dispositivo");
    for (int i = 0; i < nmon; i++) {
        DEVMODEA dm;
        cur_mode(i, &dm);
        char mode[40], pos[24], br[16];
        snprintf(mode, sizeof mode, "%lux%lu@%lu%s", dm.dmPelsWidth, dm.dmPelsHeight, dm.dmDisplayFrequency,
                 dm.dmDisplayOrientation ? (dm.dmDisplayOrientation == 1 ? " r90" : dm.dmDisplayOrientation == 2 ? " r180" : " r270") : "");
        snprintf(pos, sizeof pos, "%ld,%ld", mon[i].rc.left, mon[i].rc.top);
        if (jobs[i].ok) snprintf(br, sizeof br, "%lu%%", jobs[i].cur);
        else strcpy(br, "-");
        say("%d%c  %-20s %-16s %-12s %-8s %s\n", i + 1, mon[i].primary ? '*' : ' ', mon[i].name, mode, pos, br, mon[i].dev);
    }
    return 0;
}

HWND find_daemon(void) { return FindWindowA(WNDCLASS_NAME, NULL); }

int launch_daemon(const char *args)
{
    char exe[MAX_PATH], cmd[MAX_PATH + 32];
    snprintf(exe, sizeof exe, "%s\\kastd.exe", exedir);
    snprintf(cmd, sizeof cmd, "\"%s\" %s", exe, args ? args : "");
    STARTUPINFOA si = { .cb = sizeof si, .dwFlags = STARTF_USESHOWWINDOW, .wShowWindow = SW_SHOWNORMAL };
    PROCESS_INFORMATION pi;
    if (!CreateProcessA(exe, cmd, NULL, NULL, FALSE, DETACHED_PROCESS, NULL, exedir, &si, &pi))
        return say("no se pudo iniciar %s (%lu)\n", exe, GetLastError()), 1;
    CloseHandle(pi.hThread);
    CloseHandle(pi.hProcess);
    return 0;
}

static int cmd_post(UINT msg, const char *ok)
{
    HWND h = find_daemon();
    if (!h) return sayl("el servicio no está activo"), 1;
    PostMessageA(h, msg, 0, 0);
    if (ok) sayl(ok);
    return 0;
}

static const char HELP[] =
    "kast " KAST_VERSION " — control de monitores\n"
    "  list                        monitores (1..n de izquierda a derecha, * = principal)\n"
    "  b   [mon] [valor]           brillo     (valor: 0-100 | +10 | -10)\n"
    "  braw [mon] [valor]          brillo en la escala real del monitor (sin calibración)\n"
    "  c   [mon] [valor]           contraste\n"
    "  craw [mon] [valor]          contraste en la escala real del monitor (sin calibración)\n"
    "  vol [mon] [valor]           volumen de los altavoces del monitor\n"
    "  input <mon> [fuente]        hdmi1 hdmi2 dp1 dp2 usbc dvi1 vga1 | 0xNN\n"
    "  power <mon> on|standby|off|hardoff   energía vía DDC/CI\n"
    "  vcp <mon> <código> [valor]  código VCP crudo (0x10 o 16)\n"
    "  off                         apaga todas las pantallas (despiertan con ratón/teclado)\n"
    "  lock                        bloquea la sesión\n"
    "  mode <mon> AxB[@Hz] | @Hz   resolución / frecuencia\n"
    "  modes [mon]                 modos disponibles\n"
    "  rot <mon> 0|90|180|270      rotación\n"
    "  primary <mon>               establecer como principal\n"
    "  disable <mon>               desactivar un monitor\n"
    "  enable                      activar todos los monitores conectados\n"
    "  topo extend|clone|internal|external\n"
    "  layout [list] | layout save|load|del <nombre>   disposición de pantallas\n"
    "  fav [nombre]                ejecutar un favorito (sin nombre: listar)\n"
    "  sleep <ms>                  pausa (para secuencias)\n"
    "  ui                          abrir la interfaz gráfica\n"
    "  daemon | stop | reload | status   servicio de bandeja y atajos (kast.ini)\n"
    "  autostart on|off            arrancar el servicio con Windows\n"
    "\n"
    "mon: 1 | 1,3 | all | p (principal) | c (el que tiene el cursor)\n"
    "secuencias con ';'   kast \"b all 30; c all 50\"\n";

static int run_cmd(int ac, char **av)
{
    if (!ac) return 0;
    const char *c = av[0];
#define IS(s) (!_stricmp(c, s))
    if (IS("help") || IS("-h") || IS("/?") || IS("--help")) return say("%s", HELP), 0;
    if (IS("version") || IS("--version") || IS("-v")) return say("kast %s\n", KAST_VERSION), 0;
    if (IS("sleep")) return Sleep(ac > 1 ? atoi(av[1]) : 0), 0;
    if (IS("lock")) return !LockWorkStation();
    if (IS("off")) {
        wait_keys_released();
        PostMessageA(HWND_BROADCAST, WM_SYSCOMMAND, SC_MONITORPOWER, 2);
        return 0;
    }
    if (IS("daemon")) {
        if (find_daemon()) return sayl("el servicio ya está activo"), 0;
        return launch_daemon(NULL) || (sayl("servicio iniciado"), 0);
    }
    if (IS("ui")) return find_daemon() ? cmd_post(WM_KAST_OPENUI, NULL) : launch_daemon("--ui");
    if (IS("stop")) return cmd_post(WM_CLOSE, "servicio detenido");
    if (IS("reload")) return cmd_post(WM_KAST_RELOAD, "configuración recargada (ver kast.log)");
    if (IS("status")) return sayl(find_daemon() ? "activo" : "detenido"), !find_daemon();
    if (IS("autostart")) {
        if (ac < 2 || (_stricmp(av[1], "on") && _stricmp(av[1], "off"))) return sayl("uso: autostart on|off"), 1;
        int r = autostart_set(!_stricmp(av[1], "on"));
        return sayl(r ? "error" : "ok"), r;
    }
    if (IS("topo")) return cmd_topo(ac, av);
    if (IS("enable")) {
        int missing, cloned;
        topo_status(&missing, &cloned);
        if (!missing) return sayl("todos los monitores conectados ya están activos"), 0;
        LONG r = activate_missing();
        if (r) say("no se pudieron activar los monitores (error %ld)\n", r);
        return !!r;
    }
    if (IS("layout")) return cmd_layout(ac, av);
    if (IS("fav")) return cmd_fav(ac, av);

    if (!cfg_loaded) load_config(NULL); /* calibración */
    enum_mons();
    if (IS("list") || IS("ls")) return cmd_list();
    if (IS("b") || IS("bright")) return cmd_feature(ac, av, 0x10, 1);
    if (IS("braw")) return cmd_feature(ac, av, 0x10, 2);
    if (IS("craw")) return cmd_feature(ac, av, 0x12, 2);
    if (IS("c") || IS("contrast")) return cmd_feature(ac, av, 0x12, 1);
    if (IS("vol")) return cmd_feature(ac, av, 0x62, 1);
    if (IS("vcp")) return cmd_vcp(ac, av);
    if (IS("input")) return cmd_input(ac, av);
    if (IS("power")) return cmd_power(ac, av);
    if (IS("mode")) return cmd_mode(ac, av);
    if (IS("modes")) return cmd_modes(ac, av);
    if (IS("rot")) return cmd_rot(ac, av);
    if (IS("primary")) return cmd_primary(ac, av);
    if (IS("disable")) return cmd_disable(ac, av);
#undef IS
    say("comando desconocido: %s  (kast help)\n", c);
    return 1;
}

/* Ejecuta "cmd a b; cmd2 x" — se modifica la cadena. */
int exec_line(char *s)
{
    int rc = 0;
    for (char *seg = s, *next; seg; seg = next) {
        next = strchr(seg, ';');
        if (next) *next++ = 0;
        char *av[16];
        int ac = 0;
        for (char *p = seg; *p && ac < 16;) {
            while (isspace((unsigned char)*p)) *p++ = 0;
            if (!*p) break;
            av[ac++] = p;
            while (*p && !isspace((unsigned char)*p)) p++;
        }
        rc |= run_cmd(ac, av);
    }
    return rc;
}
