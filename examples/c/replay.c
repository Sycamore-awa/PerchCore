/* Persistent public-ABI client: context<TAB>method<TAB>JSON per UTF-8 line. */
#include "perch_core.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int check(const uint8_t *method, size_t method_len,
                 const uint8_t *args, size_t args_len, const char *expected) {
    char *result = perch_call(method, method_len, args, args_len);
    int ok = result != NULL && strstr(result, expected) != NULL;
    perch_string_free(result);
    return ok;
}

static int self_test(void) {
    const uint8_t bad[] = {0xff};
    const uint8_t method[] = "version trailing bytes";
    const uint8_t args[] = "{} trailing bytes";
    const uint8_t nul_args[] = {'{', '}', 0, 'x'};
    perch_string_free(NULL);
    if (!check(method, 7, args, 2, "\"core\":") ||
        !check(method, 7, NULL, 5, "\"core\":") ||
        !check(method, 7, bad, 0, "\"core\":") ||
        !check(NULL, 7, args, 2, "FFI_UNKNOWN_METHOD") ||
        !check(bad, sizeof bad, args, 2, "FFI_BAD_METHOD_PTR") ||
        !check(method, 7, bad, sizeof bad, "FFI_BAD_ARGS_PTR") ||
        !check(method, 7, nul_args, sizeof nul_args, "FFI_BAD_ARGS")) {
        fputs("C ABI input-length / UTF-8 / NULL contract failed\n", stderr);
        return 1;
    }
    puts("C ABI input-length / UTF-8 / NULL contract passed (7 cases)");
    return 0;
}

int main(int argc, char **argv) {
    size_t capacity = 4096, used = 0;
    char *line;
    int byte;
    if (argc == 2 && strcmp(argv[1], "--self-test") == 0) return self_test();
    if (argc != 1) return 2;
    line = malloc(capacity);
    if (line == NULL) return 1;
    while ((byte = getchar()) != EOF) {
        if (used + 1 >= capacity) {
            char *grown;
            if (capacity > (size_t)-1 / 2) { free(line); return 1; }
            capacity *= 2;
            grown = realloc(line, capacity);
            if (grown == NULL) { free(line); return 1; }
            line = grown;
        }
        if (byte != '\n') { line[used++] = (char)byte; continue; }
        if (used > 0 && line[used - 1] == '\r') --used;
        line[used] = '\0';
        {
            char *context_tab = memchr(line, '\t', used);
            char *method_tab;
            char *result;
            uint64_t context = 0;
            size_t method_len;
            if (context_tab == NULL) { free(line); return 2; }
            method_tab = memchr(context_tab + 1, '\t', used - (size_t)(context_tab + 1 - line));
            if (method_tab == NULL) { free(line); return 2; }
            method_len = (size_t)(method_tab - context_tab - 1);
            if (context_tab != line) {
                char *end;
                *context_tab = '\0';
                context = (uint64_t)strtoull(line, &end, 10);
                if (*end != '\0' || context == 0) { free(line); return 2; }
                result = perch_context_call(context,
                    (const uint8_t *)(context_tab + 1), method_len,
                    (const uint8_t *)(method_tab + 1), used - (size_t)(method_tab + 1 - line));
            } else {
                result = perch_call((const uint8_t *)(context_tab + 1), method_len,
                    (const uint8_t *)(method_tab + 1), used - (size_t)(method_tab + 1 - line));
            }
            if (result == NULL) { free(line); return 1; }
            puts(result);
            perch_string_free(result);
            if (fflush(stdout) != 0) { free(line); return 1; }
        }
        used = 0;
    }
    free(line);
    return used == 0 && !ferror(stdin) ? 0 : 2;
}
