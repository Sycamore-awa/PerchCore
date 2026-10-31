#ifndef PERCH_CORE_H
#define PERCH_CORE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Invoke a method with a UTF-8 JSON object. Lengths are in bytes.
 * Input buffers are borrowed only for the duration of this synchronous call.
 * Non-null input pointers must address at least their specified byte lengths.
 * The returned NUL-terminated UTF-8 JSON string is allocated by Rust.
 * Free it exactly once with perch_string_free(), never with free().
 * Business errors are returned as {"error":{"code":"...","message":"..."}}.
 * Treat a NULL result as a failed call.
 * Stateful methods currently share a process-wide handle registry and lock.
 */
char *perch_call(const uint8_t *method_ptr, size_t method_len,
                 const uint8_t *args_ptr, size_t args_len);

/* Accepts NULL. Otherwise ptr must be an unfreed result of perch_call(). */
void perch_string_free(char *ptr);

#ifdef __cplusplus
}
#endif

#endif
