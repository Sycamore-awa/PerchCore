#ifndef PERCH_CORE_H
#define PERCH_CORE_H

#include <stddef.h>
#include <stdint.h>

/* perch_call / perch_string_free signatures and ownership rules. */
#define PERCH_CORE_ABI_VERSION 1

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Invoke a method with a UTF-8 JSON object. Lengths are in bytes.
 * Input buffers are borrowed only for the duration of this synchronous call.
 * Non-null input pointers must address at least their specified byte lengths.
 * A NULL pointer or zero length is treated as empty input. Empty/whitespace args
 * means {}. Invalid UTF-8 produces FFI_BAD_METHOD_PTR / FFI_BAD_ARGS_PTR.
 * The returned NUL-terminated UTF-8 JSON string is allocated by Rust.
 * Free it exactly once with perch_string_free(), never with free().
 * Business errors are returned as {"error":{"code":"...","message":"..."}}.
 * Treat a NULL result as a failed call.
 * Stateful methods currently share a process-wide handle registry and lock.
 * Handles are integer IDs, including 0, shared across book/txt/pipeline kinds.
 * Current quotas: 8 book, 4 txt, 1 pipeline. Coordinate use and close.
 * Duplicate close returns closed:false; wrong-kind close does not free anything.
 * Character offsets are Unicode scalars; txtByteOffset uses decoded UTF-8 bytes.
 * See docs/ffi.md, docs/api-reference.md and api/ for all 31 method contracts.
 * Schema wrappers {method,args}/{method,result} are offline tooling only:
 * pass args itself here. Runtime does not automatically enforce those schemas.
 */
char *perch_call(const uint8_t *method_ptr, size_t method_len,
                 const uint8_t *args_ptr, size_t args_len);

/* Accepts NULL. Otherwise ptr must be an unfreed result of perch_call(). */
void perch_string_free(char *ptr);

#ifdef __cplusplus
}
#endif

#endif
