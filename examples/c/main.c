#include <stdio.h>
#include <string.h>

#include "perch_core.h"

int main(int argc, char **argv) {
    if (argc > 3) {
        fprintf(stderr, "usage: perch-example [method [args_json]]\n");
        return 2;
    }
    const char *method = argc > 1 ? argv[1] : "version";
    const char *args = argc > 2 ? argv[2] : "{}";
    char *result = perch_call((const uint8_t *)method, strlen(method),
                              (const uint8_t *)args, strlen(args));
    if (result == NULL) {
        fprintf(stderr, "perch_call returned NULL\n");
        return 1;
    }
    puts(result);
    /* This minimal client prints raw JSON; real clients should parse it. */
    perch_string_free(result);
    return 0;
}
