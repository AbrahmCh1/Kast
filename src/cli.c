/* kast.exe — interfaz de línea de comandos. */
#include "core.h"

int main(int argc, char **argv)
{
    core_init();
    SetConsoleOutputCP(CP_UTF8);
    size_t len = 8;
    for (int i = 1; i < argc; i++) len += strlen(argv[i]) + 1;
    char *line = calloc(1, len);
    for (int i = 1; i < argc; i++) {
        strcat(line, argv[i]);
        if (i + 1 < argc) strcat(line, " ");
    }
    int rc = exec_line(argc < 2 ? strcpy(line, "help") : line);
    free_phys();
    return rc;
}
